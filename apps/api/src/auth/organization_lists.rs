//! Bounded server-side organization/member/invitation lists and opt-in capacity limits.
use super::*;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;
#[derive(Default, Deserialize)]
pub(super) struct Search {
    #[serde(default)]
    q: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    page: u32,
}
async fn page(
    state: &AppState,
    org: Option<Uuid>,
    kind: &str,
    input: Search,
) -> Result<Json<Value>, ApiError> {
    if input.q.chars().count() > 120
        || input.page > 100000
        || !matches!(input.role.as_str(), "" | "owner" | "admin" | "member")
        || !matches!(
            input.status.as_str(),
            "" | "pending" | "accepted" | "expired" | "cancelled" | "rejected"
        )
    {
        return Err(bad("Invalid list filter."));
    }
    let source = match kind {
        "platform_members" => {
            "SELECT u.id,u.name,u.email,m.role,m.joined_at,auth_user_banned(u.banned,u.ban_expires_at) AS banned FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1"
        }
        "members" => {
            "SELECT u.id,u.name,u.email,m.role,m.joined_at FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1"
        }
        "invitations" => {
            "SELECT id,email,role,created_at,expires_at,accepted_at,cancelled_at,rejected_at,inviter_name,delivery_status,delivery_attempted_at,CASE WHEN accepted_at IS NOT NULL THEN 'accepted' WHEN rejected_at IS NOT NULL THEN 'rejected' WHEN cancelled_at IS NOT NULL THEN 'cancelled' WHEN expires_at<=now() THEN 'expired' ELSE 'pending' END AS status FROM auth_invitations WHERE organization_id=$1"
        }
        _ => {
            "SELECT o.id,o.name,o.logo_url,o.is_default,o.created_at,(SELECT count(*) FROM auth_members WHERE organization_id=o.id) AS member_count,(SELECT count(*) FROM projects WHERE organization_id=o.id) AS project_count FROM auth_organizations o WHERE $1::uuid IS NULL"
        }
    };
    let search = match kind {
        "members" | "platform_members" => "lower(name||' '||email)",
        "invitations" => "lower(email||' '||COALESCE(inviter_name,''))",
        _ => "lower(name)",
    };
    let role = if kind == "organizations" {
        "true"
    } else {
        "($3='' OR role=$3)"
    };
    let status = if kind == "invitations" {
        "($4='' OR status=$4)"
    } else {
        "$4=''"
    };
    let order = if kind == "invitations" {
        "created_at DESC,id DESC"
    } else {
        "name,id"
    };
    let query = format!(
        "WITH source AS ({source}), filtered AS (SELECT * FROM source WHERE ($2='' OR position(lower($2) in {search})>0) AND {role} AND {status}), paged AS (SELECT * FROM filtered ORDER BY {order} LIMIT 25 OFFSET $5) SELECT json_build_object('items',COALESCE((SELECT json_agg(paged ORDER BY {order}) FROM paged),'[]'::json),'total',(SELECT count(*) FROM filtered))::jsonb"
    );
    // Organizations has no role column but its bind must remain typed.
    let query = if kind == "organizations" {
        query.replace("AND true AND", "AND $3='' AND")
    } else {
        query
    };
    Ok(Json(
        sqlx::query_scalar::<_, Value>(&query)
            .bind(org)
            .bind(input.q.trim())
            .bind(input.role)
            .bind(input.status)
            .bind(i64::from(input.page) * 25)
            .fetch_one(&state.pool)
            .await?,
    ))
}
pub(super) async fn organizations(
    State(state): State<AppState>,
    id: Identity,
    Query(input): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    page(&state, None, "organizations", input).await
}
pub(super) async fn members(
    State(state): State<AppState>,
    id: Identity,
    Query(input): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    page(&state, Some(id.organization_id), "members", input).await
}
pub(super) async fn invitations(
    State(state): State<AppState>,
    id: Identity,
    Query(input): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    page(&state, Some(id.organization_id), "invitations", input).await
}
pub(super) async fn platform_members(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
    Query(input): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    page(&state, Some(org), "platform_members", input).await
}
pub(super) async fn platform_invitations(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
    Query(input): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    page(&state, Some(org), "invitations", input).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Limits {
    member_limit: Option<i32>,
    invitation_limit: Option<i32>,
}
async fn limits(state: &AppState, org: Uuid) -> Result<Json<Value>, ApiError> {
    let data:Option<Value>=sqlx::query_scalar("SELECT json_build_object('member_limit',member_limit,'invitation_limit',invitation_limit)::jsonb FROM auth_organizations WHERE id=$1").bind(org).fetch_optional(&state.pool).await?;
    Ok(Json(data.ok_or(ApiError(
        StatusCode::NOT_FOUND,
        "Organization not found.",
    ))?))
}
pub(super) async fn get_limits(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    limits(&state, id.organization_id).await
}
pub(super) async fn platform_limits(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    limits(&state, org).await
}
async fn save_limits(
    state: &AppState,
    id: &Identity,
    org: Uuid,
    platform: bool,
    input: Limits,
) -> Result<StatusCode, ApiError> {
    if [input.member_limit, input.invitation_limit]
        .into_iter()
        .flatten()
        .any(|n| !(1..=100000).contains(&n))
    {
        return Err(bad("Limits must be blank or between 1 and 100000."));
    }
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, id, if platform { "platform" } else { "owner" }).await?;
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_members WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    let invites:i64=sqlx::query_scalar("SELECT count(*) FROM auth_invitations WHERE organization_id=$1 AND accepted_at IS NULL AND cancelled_at IS NULL AND rejected_at IS NULL AND expires_at>now()").bind(org).fetch_one(&mut *tx).await?;
    if input.member_limit.is_some_and(|n| i64::from(n) < members)
        || input
            .invitation_limit
            .is_some_and(|n| i64::from(n) < invites)
    {
        return Err(bad(
            "A limit cannot be lower than current membership or pending invitations.",
        ));
    }
    let changed = sqlx::query(
        "UPDATE auth_organizations SET member_limit=$2,invitation_limit=$3 WHERE id=$1",
    )
    .bind(org)
    .bind(input.member_limit)
    .bind(input.invitation_limit)
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "Organization not found."));
    }
    log_target(
        &mut tx,
        id.user_id,
        org,
        if platform {
            "platform.organization_limits_updated"
        } else {
            "organization.limits_updated"
        },
        org,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn update_limits(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Limits>,
) -> Result<StatusCode, ApiError> {
    save_limits(&state, &id, id.organization_id, false, input).await
}
pub(super) async fn update_platform_limits(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
    Json(input): Json<Limits>,
) -> Result<StatusCode, ApiError> {
    save_limits(&state, &id, org, true, input).await
}
pub(super) async fn capacity(
    tx: &mut sqlx::PgConnection,
    org: Uuid,
    invitation: bool,
    user: Option<Uuid>,
) -> Result<(), ApiError> {
    // Caller holds the shared administrative lock; checks and inserts share its transaction.
    let (limit, count): (Option<i32>, i64) = if invitation {
        sqlx::query_as("SELECT invitation_limit,(SELECT count(*) FROM auth_invitations WHERE organization_id=$1 AND accepted_at IS NULL AND cancelled_at IS NULL AND rejected_at IS NULL AND expires_at>now()) FROM auth_organizations WHERE id=$1").bind(org).fetch_one(&mut *tx).await?
    } else {
        if let Some(user) = user
            && sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM auth_members WHERE organization_id=$1 AND user_id=$2)",
            )
            .bind(org)
            .bind(user)
            .fetch_one(&mut *tx)
            .await?
        {
            return Ok(());
        }
        sqlx::query_as("SELECT member_limit,(SELECT count(*) FROM auth_members WHERE organization_id=$1) FROM auth_organizations WHERE id=$1").bind(org).fetch_one(&mut *tx).await?
    };
    if limit.is_some_and(|n| count >= i64::from(n)) {
        return Err(bad(if invitation {
            "Pending invitation limit reached."
        } else {
            "Organization member limit reached."
        }));
    }
    Ok(())
}
#[derive(Deserialize)]
pub(super) struct Decline {
    email: String,
    token: String,
}
pub(super) async fn decline(
    State(state): State<AppState>,
    Json(input): Json<Decline>,
) -> Result<StatusCode, ApiError> {
    let email = accounts::email(&input.email)?;
    accounts::throttle(&state.pool, &format!("invite:{email}"), 10).await?;
    let mut tx = state.pool.begin().await?;
    management::lock(&mut tx).await?;
    let found:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT id,organization_id FROM auth_invitations WHERE email=$1 AND token_hash=$2 AND accepted_at IS NULL AND rejected_at IS NULL AND cancelled_at IS NULL AND expires_at>now() FOR UPDATE").bind(&email).bind(digest(&input.token)).fetch_optional(&mut *tx).await?;
    let (invite, org) = found.ok_or_else(invalid)?;
    sqlx::query("UPDATE auth_invitations SET rejected_at=now(),expires_at=now() WHERE id=$1")
        .bind(invite)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action,target_id,actor_name) VALUES($1,(SELECT id FROM auth_users WHERE email=$2),$3,'invitation.rejected',$4,'Invitation recipient')").bind(Uuid::new_v4()).bind(email).bind(org).bind(invite).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

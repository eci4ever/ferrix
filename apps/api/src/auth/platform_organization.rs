//! Platform organization administration never switches sessions or grants membership.
use super::*;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

async fn exists(tx: &mut sqlx::PgConnection, org: Uuid) -> Result<(), ApiError> {
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM auth_organizations WHERE id=$1)")
        .bind(org)
        .fetch_one(tx)
        .await?
    {
        return Err(ApiError(StatusCode::NOT_FOUND, "Organization not found."));
    }
    Ok(())
}
pub(super) async fn detail(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    id.admin()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    exists(&mut tx, org).await?;
    let organization:Value=sqlx::query_scalar("SELECT json_build_object('id',o.id,'name',o.name,'logo_url',o.logo_url,'is_default',o.is_default,'created_at',o.created_at,'member_count',(SELECT count(*) FROM auth_members WHERE organization_id=o.id),'project_count',(SELECT count(*) FROM projects WHERE organization_id=o.id))::jsonb FROM auth_organizations o WHERE id=$1").bind(org).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"organization":organization})),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Role {
    role: String,
}
pub(super) async fn role(
    State(state): State<AppState>,
    id: Identity,
    Path((org, target)): Path<(Uuid, Uuid)>,
    Json(input): Json<Role>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    if !matches!(input.role.as_str(), "owner" | "admin" | "member") {
        return Err(bad("Choose owner, admin or member."));
    }
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    exists(&mut tx, org).await?;
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1 AND m.user_id=$2 AND NOT auth_user_banned(u.banned,u.ban_expires_at))").bind(org).bind(target).fetch_one(&mut *tx).await?;
    if !active {
        return Err(bad("Choose an active member of this organization."));
    }
    organizations::preserve_owner(&mut tx, org, target, input.role == "owner").await?;
    sqlx::query("UPDATE auth_members SET role=$1 WHERE organization_id=$2 AND user_id=$3")
        .bind(input.role)
        .bind(org)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    log_target(
        &mut tx,
        id.user_id,
        org,
        "platform.member_role_changed",
        target,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path((org, target)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    if org == id.organization_id && target == id.user_id {
        return Err(bad(
            "Use Organization access to leave your active organization.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    exists(&mut tx, org).await?;
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM auth_members WHERE organization_id=$1 AND user_id=$2)",
    )
    .bind(org)
    .bind(target)
    .fetch_one(&mut *tx)
    .await?;
    if !present {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Member not found in this organization.",
        ));
    }
    organizations::preserve_owner(&mut tx, org, target, false).await?;
    management::preserve_platform_access(&mut tx, target, org).await?;
    sqlx::query("DELETE FROM auth_members WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE active_organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    log_target(&mut tx, id.user_id, org, "platform.member_removed", target).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Invitation {
    email: String,
    role: String,
}
pub(super) async fn invite(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
    Json(input): Json<Invitation>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    id.fresh()?;
    if !matches!(input.role.as_str(), "admin" | "member") {
        return Err(bad("Choose admin or member."));
    }
    let email = accounts::email(&input.email)?;
    accounts::throttle(&state.pool, &format!("invitation-email:{}", id.user_id), 30).await?;
    let value = token();
    let invite = Uuid::new_v4();
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    exists(&mut tx, org).await?;
    sqlx::query("UPDATE auth_invitations SET expires_at=now(),cancelled_at=now() WHERE organization_id=$1 AND email=$2 AND accepted_at IS NULL AND cancelled_at IS NULL AND rejected_at IS NULL AND expires_at>now()").bind(org).bind(&email).execute(&mut *tx).await?;
    organization_lists::capacity(&mut tx, org, true, None).await?;
    sqlx::query("INSERT INTO auth_invitations(id,organization_id,email,role,token_hash,invited_by) VALUES($1,$2,$3,$4,$5,$6)").bind(invite).bind(org).bind(&email).bind(&input.role).bind(digest(&value)).bind(id.user_id).execute(&mut *tx).await?;
    log_target(
        &mut tx,
        id.user_id,
        org,
        "platform.invitation_created",
        invite,
    )
    .await?;
    tx.commit().await?;
    let delivery =
        invitation_email::deliver(&state, invite, org, &email, &input.role, &value).await;
    Ok(Json(
        json!({"id":invite,"token":value,"expires_in_seconds":172800,"email_delivery":delivery}),
    ))
}
pub(super) async fn renew(
    State(state): State<AppState>,
    id: Identity,
    Path((org, invite)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    id.fresh()?;
    accounts::throttle(&state.pool, &format!("invitation-email:{}", id.user_id), 30).await?;
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    exists(&mut tx, org).await?;
    let (email,role):(String,String)=sqlx::query_as("SELECT email,role FROM auth_invitations WHERE id=$1 AND organization_id=$2 AND accepted_at IS NULL AND cancelled_at IS NULL AND rejected_at IS NULL").bind(invite).bind(org).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"Invitation not found in this organization."))?;
    let value = token();
    sqlx::query("UPDATE auth_invitations SET expires_at=now() WHERE id=$1")
        .bind(invite)
        .execute(&mut *tx)
        .await?;
    organization_lists::capacity(&mut tx, org, true, None).await?;
    sqlx::query("UPDATE auth_invitations SET token_hash=$1,expires_at=now()+interval '2 days',invited_by=$2,delivery_status='unknown',delivery_attempted_at=NULL WHERE id=$3").bind(digest(&value)).bind(id.user_id).bind(invite).execute(&mut *tx).await?;
    log_target(
        &mut tx,
        id.user_id,
        org,
        "platform.invitation_renewed",
        invite,
    )
    .await?;
    tx.commit().await?;
    let delivery = invitation_email::deliver(&state, invite, org, &email, &role, &value).await;
    Ok(Json(
        json!({"token":value,"expires_in_seconds":172800,"email_delivery":delivery}),
    ))
}
pub(super) async fn revoke(
    State(state): State<AppState>,
    id: Identity,
    Path((org, invite)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    exists(&mut tx, org).await?;
    let result=sqlx::query("UPDATE auth_invitations SET expires_at=now(),cancelled_at=now() WHERE id=$1 AND organization_id=$2 AND accepted_at IS NULL AND cancelled_at IS NULL AND rejected_at IS NULL AND expires_at>now()").bind(invite).bind(org).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Invitation not found in this organization.",
        ));
    }
    log_target(
        &mut tx,
        id.user_id,
        org,
        "platform.invitation_revoked",
        invite,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

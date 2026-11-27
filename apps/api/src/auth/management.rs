//! Administrative transitions share a short transaction lock. Recheck authority
//! inside that lock so simultaneous removals/demotions cannot strand an installation.
use super::*;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

pub(super) async fn lock(tx: &mut sqlx::PgConnection) -> Result<(), ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(1487088941)")
        .execute(tx)
        .await?;
    Ok(())
}
pub(super) async fn authorize(
    tx: &mut sqlx::PgConnection,
    id: &Identity,
    access: &str,
) -> Result<String, ApiError> {
    id.fresh()?;
    lock(tx).await?;
    let row: Option<(String,String)> = sqlx::query_as("SELECT u.system_role,m.role FROM auth_users u JOIN auth_members m ON m.user_id=u.id AND m.organization_id=$2 JOIN auth_sessions s ON s.user_id=u.id AND s.id=$3 AND s.active_organization_id=m.organization_id WHERE u.id=$1 AND NOT auth_user_banned(u.banned,u.ban_expires_at) AND s.expires_at>now() AND s.absolute_expires_at>now() AND s.authenticated_at>now()-interval '15 minutes'")
        .bind(id.user_id).bind(id.organization_id).bind(id.session_id).fetch_optional(tx).await?;
    let (system, role) = row.ok_or_else(forbidden)?;
    if match access {
        "platform" => system == "admin",
        "owner" => role == "owner",
        "manager" => matches!(role.as_str(), "owner" | "admin"),
        "member" => true,
        _ => false,
    } {
        Ok(role)
    } else {
        Err(forbidden())
    }
}
#[derive(Deserialize, Default)]
pub struct Search {
    #[serde(default)]
    q: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    page: u32,
    limit: Option<u32>,
}
#[derive(Serialize, FromRow)]
struct UserRow {
    id: Uuid,
    name: String,
    email: String,
    system_role: String,
    banned: bool,
    ban_reason: Option<String>,
    ban_expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}
pub async fn users(
    State(state): State<AppState>,
    id: Identity,
    Query(input): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    if input.q.len() > 254
        || !matches!(input.role.as_str(), "" | "admin" | "user")
        || !matches!(input.status.as_str(), "" | "active" | "banned")
        || input.page > 100_000
    {
        return Err(bad("Invalid user search."));
    }
    let limit = input.limit.unwrap_or(10);
    if !(1..=50).contains(&limit) {
        return Err(bad("Choose a page size between 1 and 50."));
    }
    // Literal substring search: user wildcard characters do not alter the query.
    let filter = " WHERE ($1='' OR strpos(lower(name),lower($1))>0 OR strpos(email,lower($1))>0) AND ($2='' OR system_role=$2) AND ($3='' OR ($3='banned')=auth_user_banned(banned,ban_expires_at))";
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM auth_users{filter}"))
        .bind(input.q.trim())
        .bind(&input.role)
        .bind(&input.status)
        .fetch_one(&mut *tx)
        .await?;
    let rows:Vec<UserRow>=sqlx::query_as(&format!("SELECT id,name,email,system_role,auth_user_banned(banned,ban_expires_at) banned,ban_reason,ban_expires_at,created_at FROM auth_users{filter} ORDER BY created_at DESC,id LIMIT $4 OFFSET $5"))
        .bind(input.q.trim()).bind(&input.role).bind(&input.status).bind(i64::from(limit)).bind(i64::from(input.page)*i64::from(limit)).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"items":rows,"total":total,"page":input.page,"limit":limit}),
    ))
}
#[derive(Serialize, FromRow)]
struct UserSession {
    id: Uuid,
    user_agent: String,
    ip_address: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    current: bool,
}
pub async fn user_detail(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    let user:UserRow=sqlx::query_as("SELECT id,name,email,system_role,auth_user_banned(banned,ban_expires_at) banned,ban_reason,ban_expires_at,created_at FROM auth_users WHERE id=$1").bind(target).fetch_optional(&state.pool).await?.ok_or_else(||bad("User not found."))?;
    let organizations:Vec<(Uuid,String,String,Option<DateTime<Utc>>) >=sqlx::query_as("SELECT o.id,o.name,m.role,m.joined_at FROM auth_members m JOIN auth_organizations o ON o.id=m.organization_id WHERE m.user_id=$1 ORDER BY o.name,o.id").bind(target).fetch_all(&state.pool).await?;
    // Never expose authentication tokens, IP trust headers, passwords or MFA secrets.
    let sessions: Vec<UserSession> = sqlx::query_as("SELECT id,user_agent,ip_address,created_at,expires_at,(id=$2) current FROM auth_sessions WHERE user_id=$1 AND expires_at>now() AND absolute_expires_at>now() ORDER BY created_at DESC LIMIT 100")
        .bind(target).bind(id.session_id).fetch_all(&state.pool).await?;
    let summary:Value=sqlx::query_scalar("SELECT json_build_object('last_sign_in_at',u.last_sign_in_at,'mfa_enabled',EXISTS(SELECT 1 FROM auth_mfa WHERE user_id=u.id AND enabled),'api_keys',json_build_object('active',(SELECT count(*) FROM auth_api_keys WHERE user_id=u.id AND revoked_at IS NULL AND expires_at>now()),'revoked',(SELECT count(*) FROM auth_api_keys WHERE user_id=u.id AND revoked_at IS NOT NULL),'expired',(SELECT count(*) FROM auth_api_keys WHERE user_id=u.id AND revoked_at IS NULL AND expires_at<=now())))::jsonb FROM auth_users u WHERE u.id=$1").bind(target).fetch_one(&state.pool).await?;
    Ok(Json(
        json!({"summary":summary,"user":user,"organizations":organizations.into_iter().map(|(id,name,role,joined_at)|json!({"id":id,"name":name,"role":role,"joined_at":joined_at})).collect::<Vec<_>>(),"sessions":sessions}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ban {
    banned: bool,
    #[serde(default)]
    reason: String,
    expires_at: Option<DateTime<Utc>>,
}
pub async fn ban(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
    Json(input): Json<Ban>,
) -> Result<StatusCode, ApiError> {
    if target == id.user_id {
        return Err(bad("You cannot ban your own account."));
    }
    if (input.banned && input.reason.trim().is_empty())
        || input.reason.chars().count() > 500
        || input.expires_at.is_some_and(|expiry| {
            expiry <= Utc::now() || expiry > Utc::now() + chrono::Duration::days(365)
        })
    {
        return Err(bad(
            "Provide a reason up to 500 characters and a future expiry within one year.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "platform").await?;
    let count=sqlx::query("UPDATE auth_users SET banned=$1,ban_reason=$3,ban_expires_at=$4 WHERE id=$2 AND system_role<>'admin' AND (NOT $1 OR NOT EXISTS(SELECT 1 FROM auth_members WHERE user_id=$2 AND role='owner'))")
        .bind(input.banned).bind(target).bind(if input.banned{Some(input.reason.trim())}else{None}).bind(if input.banned{input.expires_at}else{None}).execute(&mut *tx).await?.rows_affected();
    if count == 0 {
        return Err(forbidden());
    }
    if input.banned {
        sqlx::query(
            "UPDATE auth_api_keys SET revoked_at=COALESCE(revoked_at,now()) WHERE user_id=$1",
        )
        .bind(target)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1")
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    log_target(
        &mut tx,
        id.user_id,
        id.organization_id,
        if input.banned {
            "admin.user_banned"
        } else {
            "admin.user_unbanned"
        },
        target,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserRole {
    role: String,
}
async fn preserve_admin(tx: &mut sqlx::PgConnection, target: Uuid) -> Result<(), ApiError> {
    let last:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users WHERE id=$1 AND system_role='admin') AND NOT EXISTS(SELECT 1 FROM auth_users WHERE id<>$1 AND system_role='admin' AND NOT auth_user_banned(banned,ban_expires_at) AND EXISTS(SELECT 1 FROM auth_members WHERE user_id=auth_users.id))").bind(target).fetch_one(tx).await?;
    if last {
        Err(bad(
            "The installation must retain an active platform administrator.",
        ))
    } else {
        Ok(())
    }
}
pub async fn user_role(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
    Json(input): Json<UserRole>,
) -> Result<StatusCode, ApiError> {
    if !matches!(input.role.as_str(), "admin" | "user") {
        return Err(bad("Choose admin or user."));
    }
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "platform").await?;
    if input.role == "user" {
        preserve_admin(&mut tx, target).await?;
    }
    let n=sqlx::query("UPDATE auth_users SET system_role=$1 WHERE id=$2 AND NOT auth_user_banned(banned,ban_expires_at) AND ($1<>'admin' OR EXISTS(SELECT 1 FROM auth_members WHERE user_id=auth_users.id))").bind(input.role).bind(target).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(bad(
            "User not found or banned. Unban before changing platform role.",
        ));
    }
    log_target(
        &mut tx,
        id.user_id,
        id.organization_id,
        "admin.user_role_changed",
        target,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    confirmation: String,
}
pub async fn delete_user(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
    Json(input): Json<Confirmation>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "platform").await?;
    if target == id.user_id {
        return Err(bad("You cannot delete your own account."));
    }
    preserve_admin(&mut tx, target).await?;
    let email: Option<String> =
        sqlx::query_scalar("SELECT email FROM auth_users WHERE id=$1 FOR UPDATE")
            .bind(target)
            .fetch_optional(&mut *tx)
            .await?;
    if email.as_deref() != Some(&input.confirmation) {
        return Err(bad("Enter the user's exact email to confirm deletion."));
    }
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM auth_members WHERE user_id=$1 AND role='owner')",
    )
    .bind(target)
    .fetch_one(&mut *tx)
    .await?;
    if owns {
        return Err(bad(
            "Transfer all organization ownership before deleting this user.",
        ));
    }
    // Do not turn pending user requests into trusted system jobs when the FK is cleared.
    sqlx::query("UPDATE deployments SET status='failed',finished_at=now(),logs=left(logs || E'Requesting account deleted before deployment started.\\n',65536) WHERE requested_by=$1 AND status='queued'")
        .bind(target)
        .execute(&mut *tx)
        .await?;
    let running: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM deployments WHERE requested_by=$1 AND status='running')",
    )
    .bind(target)
    .fetch_one(&mut *tx)
    .await?;
    if running {
        return Err(bad(
            "Wait for this user's running deployments to finish before deleting the account.",
        ));
    }
    // Retire invitations issued by the deleted account before removing its FK target.
    sqlx::query("UPDATE auth_invitations SET cancelled_at=COALESCE(cancelled_at,now()),expires_at=LEAST(expires_at,now()) WHERE invited_by=$1 AND accepted_at IS NULL AND rejected_at IS NULL AND cancelled_at IS NULL AND expires_at>now()")
        .bind(target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_users WHERE id=$1")
        .bind(target)
        .execute(&mut *tx)
        .await?;
    log_target(
        &mut tx,
        id.user_id,
        id.organization_id,
        "admin.user_deleted",
        target,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn organization(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Value>, ApiError> {
    let (name, logo_url, is_default): (String, Option<String>, bool) =
        sqlx::query_as("SELECT name,logo_url,is_default FROM auth_organizations WHERE id=$1")
            .bind(id.organization_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(
        json!({"id":id.organization_id,"name":name,"logo_url":logo_url,"is_default":is_default}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationInput {
    name: String,
    logo_url: Option<String>,
}
fn organization_values(input: OrganizationInput) -> Result<(String, Option<String>), ApiError> {
    let name = accounts::name(&input.name)?;
    let logo = input
        .logo_url
        .filter(|url| !url.trim().is_empty())
        .map(|url| {
            let url = url.trim();
            let parsed = url::Url::parse(url).map_err(|_| bad("Use an HTTPS logo URL."))?;
            if url.len() > 2048
                || parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(bad("Use an HTTPS logo URL without credentials."));
            }
            Ok(url.to_owned())
        })
        .transpose()?;
    Ok((name, logo))
}
pub async fn update_organization(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<OrganizationInput>,
) -> Result<StatusCode, ApiError> {
    let (name, logo) = organization_values(input)?;
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "manager").await?;
    sqlx::query("UPDATE auth_organizations SET name=$1,logo_url=$2 WHERE id=$3")
        .bind(name)
        .bind(logo)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "organization.updated",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    user_id: Uuid,
}
pub async fn transfer(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Transfer>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "owner").await?;
    if input.user_id == id.user_id {
        return Err(bad("Choose another active member."));
    }
    let n=sqlx::query("UPDATE auth_members m SET role='owner' FROM auth_users u WHERE m.organization_id=$1 AND m.user_id=$2 AND u.id=m.user_id AND NOT auth_user_banned(u.banned,u.ban_expires_at)").bind(id.organization_id).bind(input.user_id).execute(&mut *tx).await?.rows_affected();
    if n != 1 {
        return Err(bad("Choose an active member of this organization."));
    }
    sqlx::query("UPDATE auth_members SET role='admin' WHERE organization_id=$1 AND user_id=$2")
        .bind(id.organization_id)
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "organization.ownership_transferred",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn preserve_platform_access(
    tx: &mut sqlx::PgConnection,
    user: Uuid,
    org: Uuid,
) -> Result<(), ApiError> {
    let strands: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users WHERE id=$1 AND system_role='admin') AND NOT EXISTS(SELECT 1 FROM auth_members WHERE user_id=$1 AND organization_id<>$2)")
        .bind(user).bind(org).fetch_one(tx).await?;
    if strands {
        Err(bad(
            "Platform administrators must join another organization before this membership can be removed.",
        ))
    } else {
        Ok(())
    }
}
pub async fn leave(State(state): State<AppState>, id: Identity) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "member").await?;
    let last:bool=sqlx::query_scalar("SELECT (SELECT role FROM auth_members WHERE organization_id=$1 AND user_id=$2)='owner' AND (SELECT count(*) FROM auth_members WHERE organization_id=$1 AND role='owner')<=1").bind(id.organization_id).bind(id.user_id).fetch_one(&mut *tx).await?;
    if last {
        return Err(bad("Transfer ownership before leaving the last owner."));
    }
    preserve_platform_access(&mut tx, id.user_id, id.organization_id).await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1 AND active_organization_id=$2")
        .bind(id.user_id)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_members WHERE user_id=$1 AND organization_id=$2")
        .bind(id.user_id)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, id.user_id, id.organization_id, "organization.left").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn delete_organization(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Confirmation>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "owner").await?;
    let (name, is_default): (String, bool) =
        sqlx::query_as("SELECT name,is_default FROM auth_organizations WHERE id=$1 FOR UPDATE")
            .bind(id.organization_id)
            .fetch_one(&mut *tx)
            .await?;
    if is_default {
        return Err(bad("The default organization cannot be deleted."));
    }
    if input.confirmation != name {
        return Err(bad(
            "Enter the exact organization name to confirm deletion.",
        ));
    }
    let strands_admin: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1 AND u.system_role='admin' AND NOT EXISTS(SELECT 1 FROM auth_members other WHERE other.user_id=u.id AND other.organization_id<>$1))")
        .bind(id.organization_id).fetch_one(&mut *tx).await?;
    if strands_admin {
        return Err(bad(
            "Platform administrators must join another organization before this one is deleted.",
        ));
    }
    let projects: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE organization_id=$1)")
            .bind(id.organization_id)
            .fetch_one(&mut *tx)
            .await?;
    if projects {
        return Err(bad("Delete all projects before deleting the organization."));
    }
    // Move only the requesting session before the FK cascade revokes sessions
    // still using the deleted organization. Never grant a new membership or
    // extend authentication freshness or session expiry during this transition.
    let fallback: Option<Uuid> = sqlx::query_scalar(
        "SELECT o.id FROM auth_organizations o JOIN auth_members m ON m.organization_id=o.id WHERE m.user_id=$1 AND o.id<>$2 ORDER BY o.is_default DESC,o.created_at,o.id LIMIT 1",
    )
    .bind(id.user_id)
    .bind(id.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(organization) = fallback {
        sqlx::query("UPDATE auth_sessions SET active_organization_id=$1 WHERE id=$2 AND user_id=$3 AND active_organization_id=$4")
            .bind(organization)
            .bind(id.session_id)
            .bind(id.user_id)
            .bind(id.organization_id)
            .execute(&mut *tx)
            .await?;
    }
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "organization.deleted",
    )
    .await?;
    sqlx::query("DELETE FROM auth_organizations WHERE id=$1")
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn renew_invitation(
    State(state): State<AppState>,
    id: Identity,
    Path(invite): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    id.fresh()?;
    accounts::throttle(&state.pool, &format!("invitation-email:{}", id.user_id), 30).await?;
    let mut tx = state.pool.begin().await?;
    let role = authorize(&mut tx, &id, "manager").await?;
    let invited:Option<(String,String)>=sqlx::query_as("SELECT role,email FROM auth_invitations WHERE id=$1 AND organization_id=$2 AND accepted_at IS NULL AND cancelled_at IS NULL AND rejected_at IS NULL").bind(invite).bind(id.organization_id).fetch_optional(&mut *tx).await?;
    if invited.is_none() {
        return Err(ApiError(StatusCode::NOT_FOUND, "Invitation not found."));
    }
    match invited.as_ref().map(|(role, _)| role.as_str()) {
        Some("member") => {}
        Some("admin") if role == "owner" => {}
        _ => return Err(forbidden()),
    }
    let value = token();
    sqlx::query("UPDATE auth_invitations SET expires_at=now() WHERE id=$1")
        .bind(invite)
        .execute(&mut *tx)
        .await?;
    organization_lists::capacity(&mut tx, id.organization_id, true, None).await?;
    sqlx::query("UPDATE auth_invitations SET token_hash=$1,expires_at=now()+interval '2 days',invited_by=$2,delivery_status='unknown',delivery_attempted_at=NULL WHERE id=$3").bind(digest(&value)).bind(id.user_id).bind(invite).execute(&mut *tx).await?;
    log_target(
        &mut tx,
        id.user_id,
        id.organization_id,
        "invitation.renewed",
        invite,
    )
    .await?;
    tx.commit().await?;
    let (role, email) = invited.unwrap();
    let delivery =
        invitation_email::deliver(&state, invite, id.organization_id, &email, &role, &value).await;
    Ok(Json(
        json!({"token":value,"expires_in_seconds":172800,"email_delivery":delivery}),
    ))
}

#[derive(Serialize, FromRow)]
struct PlatformOrganization {
    id: Uuid,
    name: String,
    logo_url: Option<String>,
    is_default: bool,
    created_at: DateTime<Utc>,
    member_count: i64,
    project_count: i64,
}
// Platform administrators manage organization metadata without acquiring membership.
pub async fn all_organizations(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    let rows: Vec<PlatformOrganization> = sqlx::query_as("SELECT o.id,o.name,o.logo_url,o.is_default,o.created_at,(SELECT count(*) FROM auth_members m WHERE m.organization_id=o.id) AS member_count,(SELECT count(*) FROM projects p WHERE p.organization_id=o.id) AS project_count FROM auth_organizations o ORDER BY o.created_at,o.id").fetch_all(&state.pool).await?;
    Ok(Json(json!(rows)))
}
pub async fn platform_update_organization(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
    Json(input): Json<OrganizationInput>,
) -> Result<StatusCode, ApiError> {
    let (name, logo) = organization_values(input)?;
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "platform").await?;
    let result = sqlx::query("UPDATE auth_organizations SET name=$1,logo_url=$2 WHERE id=$3")
        .bind(name)
        .bind(logo)
        .bind(org)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "Organization not found."));
    }
    log(&mut tx, id.user_id, org, "platform.organization_updated").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn platform_delete_organization(
    State(state): State<AppState>,
    id: Identity,
    Path(org): Path<Uuid>,
    Json(input): Json<Confirmation>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    authorize(&mut tx, &id, "platform").await?;
    let row: Option<(String, bool)> =
        sqlx::query_as("SELECT name,is_default FROM auth_organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_optional(&mut *tx)
            .await?;
    let (name, is_default) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "Organization not found."))?;
    if is_default {
        return Err(bad("The default organization cannot be deleted."));
    }
    if org == id.organization_id {
        return Err(bad(
            "Switch to another organization before deleting your active organization.",
        ));
    }
    if input.confirmation != name {
        return Err(bad(
            "Enter the exact organization name to confirm deletion.",
        ));
    }
    let projects: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE organization_id=$1)")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if projects {
        return Err(bad("Delete all projects before deleting the organization."));
    }
    let stranded:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1 AND u.system_role='admin' AND NOT EXISTS(SELECT 1 FROM auth_members other WHERE other.user_id=u.id AND other.organization_id<>$1))").bind(org).fetch_one(&mut *tx).await?;
    if stranded {
        return Err(bad(
            "Platform administrators must join another organization before this one is deleted.",
        ));
    }
    log(&mut tx, id.user_id, org, "platform.organization_deleted").await?;
    sqlx::query("DELETE FROM auth_organizations WHERE id=$1")
        .bind(org)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

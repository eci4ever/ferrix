use super::*;
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn list(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows:Vec<(Uuid,String,String,Option<String>)>=sqlx::query_as("SELECT o.id,o.name,m.role,o.logo_url FROM auth_organizations o JOIN auth_members m ON m.organization_id=o.id WHERE m.user_id=$1 ORDER BY o.created_at").bind(id.user_id).fetch_all(&state.pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, role, logo_url)| json!({"id":id,"name":name,"role":role,"logo_url":logo_url}))
            .collect(),
    ))
}
#[derive(Deserialize)]
pub struct Organization {
    name: String,
}
pub async fn create(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Organization>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    id.fresh()?;
    let name = accounts::name(&input.name)?;
    let org = Uuid::new_v4();
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,$2)")
        .bind(org)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'owner')")
        .bind(org)
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, id.user_id, org, "organization.created").await?;
    tx.commit().await?;
    Ok(Json(json!({"id":org,"name":name})))
}
#[derive(Deserialize)]
pub struct Switch {
    organization_id: Uuid,
}
pub async fn switch(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Switch>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    let count=sqlx::query("UPDATE auth_sessions SET active_organization_id=$1 WHERE id=$2 AND EXISTS(SELECT 1 FROM auth_members WHERE organization_id=$1 AND user_id=$3)").bind(input.organization_id).bind(id.session_id).bind(id.user_id).execute(&mut *tx).await?.rows_affected();
    if count == 0 {
        return Err(forbidden());
    }
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "organization.switched",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn members(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows:Vec<(Uuid,String,String,String)>=sqlx::query_as("SELECT u.id,u.name,u.email,m.role FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1 ORDER BY u.name").bind(id.organization_id).fetch_all(&state.pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, email, role)| json!({"id":id,"name":name,"email":email,"role":role}))
            .collect(),
    ))
}
#[derive(Deserialize)]
pub struct Invitation {
    email: String,
    role: String,
}
pub async fn invite(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Invitation>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    id.fresh()?;
    if !matches!(input.role.as_str(), "admin" | "member")
        || (input.role == "admin" && id.role != "owner")
    {
        return Err(forbidden());
    }
    let email = accounts::email(&input.email)?;
    accounts::throttle(&state.pool, &format!("invitation-email:{}", id.user_id), 30).await?;
    let token = token();
    let invite = Uuid::new_v4();
    let mut tx = state.pool.begin().await?;
    let live_role = management::authorize(&mut tx, &id, "manager").await?;
    if input.role == "admin" && live_role != "owner" {
        return Err(forbidden());
    }
    sqlx::query("UPDATE auth_invitations SET expires_at=now() WHERE organization_id=$1 AND email=$2 AND accepted_at IS NULL").bind(id.organization_id).bind(&email).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_invitations(id,organization_id,email,role,token_hash,invited_by) VALUES($1,$2,$3,$4,$5,$6)").bind(invite).bind(id.organization_id).bind(&email).bind(&input.role).bind(digest(&token)).bind(id.user_id).execute(&mut *tx).await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "invitation.created",
    )
    .await?;
    tx.commit().await?;
    let delivery = invitation_email::deliver(
        &state,
        invite,
        id.organization_id,
        &email,
        &input.role,
        &token,
    )
    .await;
    Ok(Json(
        json!({"id":invite,"token":token,"expires_in_seconds":172800,"email_delivery":delivery}),
    ))
}
pub async fn invitations(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Vec<Value>>, ApiError> {
    id.manager()?;
    let rows:Vec<(Uuid,String,String,chrono::DateTime<chrono::Utc>)>=sqlx::query_as("SELECT id,email,role,expires_at FROM auth_invitations WHERE organization_id=$1 AND accepted_at IS NULL AND expires_at>now() ORDER BY created_at DESC,id").bind(id.organization_id).fetch_all(&state.pool).await?;
    Ok(Json(rows.into_iter().map(|(id,email,role,expires_at)|json!({"id":id,"email":email,"role":role,"expires_at":expires_at})).collect()))
}
pub async fn revoke_invite(
    State(state): State<AppState>,
    id: Identity,
    Path(invite): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    let live_role = management::authorize(&mut tx, &id, "manager").await?;
    let invited_role: Option<String> =
        sqlx::query_scalar("SELECT role FROM auth_invitations WHERE id=$1 AND organization_id=$2")
            .bind(invite)
            .bind(id.organization_id)
            .fetch_optional(&mut *tx)
            .await?;
    if live_role != "owner" && invited_role.as_deref() != Some("member") {
        return Err(forbidden());
    }
    sqlx::query("UPDATE auth_invitations SET expires_at=now() WHERE id=$1 AND organization_id=$2")
        .bind(invite)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "invitation.revoked",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct Accept {
    #[serde(flatten)]
    account: accounts::Registration,
    token: String,
}
pub async fn accept(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Accept>,
) -> Result<Response, ApiError> {
    let email = accounts::email(&input.account.email)?;
    accounts::throttle(&state.pool, &format!("invite:{email}"), 10).await?;
    let name = accounts::name(&input.account.name)?;
    let existing: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id,password_hash,auth_user_banned(banned,ban_expires_at) FROM auth_users WHERE email=$1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await?;
    let hash = if let Some((_, hash, banned)) = &existing {
        accounts::verify(input.account.password, Some(hash.clone())).await?;
        if *banned {
            return Err(invalid());
        }
        hash.clone()
    } else {
        accounts::hash(input.account.password).await?
    };
    let mut tx = state.pool.begin().await?;
    management::lock(&mut tx).await?;
    let invitation:Option<(Uuid,Uuid,String)>=sqlx::query_as("SELECT id,organization_id,role FROM auth_invitations WHERE token_hash=$1 AND email=$2 AND accepted_at IS NULL AND expires_at>now() FOR UPDATE").bind(digest(&input.token)).bind(&email).fetch_optional(&mut *tx).await?;
    let (invite, org, role) = invitation.ok_or_else(invalid)?;
    let user = if let Some((user, _, _)) = existing {
        let current: Option<(String, bool)> =
            sqlx::query_as("SELECT password_hash,auth_user_banned(banned,ban_expires_at) FROM auth_users WHERE id=$1 FOR UPDATE")
                .bind(user)
                .fetch_optional(&mut *tx)
                .await?;
        if current != Some((hash, false)) {
            return Err(invalid());
        }
        security::verify_mfa(&mut tx, &state, user, &input.account.code).await?;
        user
    } else {
        let user = Uuid::new_v4();
        sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) VALUES($1,$2,$3,$4)")
            .bind(user)
            .bind(name)
            .bind(email)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        user
    };
    // Existing membership is never overwritten by an invitation with a weaker role.
    sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(org).bind(user).bind(role).execute(&mut *tx).await?;
    sqlx::query("UPDATE auth_invitations SET accepted_at=now() WHERE id=$1")
        .bind(invite)
        .execute(&mut *tx)
        .await?;
    let token = sessions::issue(&mut tx, user, org, &headers).await?;
    log(&mut tx, user, org, "invitation.accepted").await?;
    tx.commit().await?;
    Ok(sessions::signed_in(&state, &token))
}
#[derive(Deserialize)]
pub struct Role {
    role: String,
}
pub async fn role(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
    Json(input): Json<Role>,
) -> Result<StatusCode, ApiError> {
    id.fresh()?;
    if id.role != "owner" || !matches!(input.role.as_str(), "owner" | "admin" | "member") {
        return Err(forbidden());
    }
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "owner").await?;
    lock_org(&mut tx, id.organization_id).await?;
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1 AND m.user_id=$2 AND NOT auth_user_banned(u.banned,u.ban_expires_at))").bind(id.organization_id).bind(target).fetch_one(&mut *tx).await?;
    if !active {
        return Err(bad("Choose an active organization member."));
    }
    preserve_owner(&mut tx, id.organization_id, target, input.role == "owner").await?;
    sqlx::query("UPDATE auth_members SET role=$1 WHERE user_id=$2 AND organization_id=$3")
        .bind(input.role)
        .bind(target)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    log_target(
        &mut tx,
        id.user_id,
        id.organization_id,
        "member.role_changed",
        target,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    id.fresh()?;
    let mut tx = state.pool.begin().await?;
    let live_role = management::authorize(&mut tx, &id, "manager").await?;
    lock_org(&mut tx, id.organization_id).await?;
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM auth_members WHERE organization_id=$1 AND user_id=$2")
            .bind(id.organization_id)
            .bind(target)
            .fetch_optional(&mut *tx)
            .await?;
    if live_role != "owner" && role.as_deref() != Some("member") {
        return Err(forbidden());
    }
    preserve_owner(&mut tx, id.organization_id, target, false).await?;
    management::preserve_platform_access(&mut tx, target, id.organization_id).await?;
    sqlx::query("DELETE FROM auth_members WHERE organization_id=$1 AND user_id=$2")
        .bind(id.organization_id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE active_organization_id=$1 AND user_id=$2")
        .bind(id.organization_id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, id.user_id, id.organization_id, "member.removed").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn lock_org(tx: &mut sqlx::PgConnection, org: Uuid) -> Result<(), ApiError> {
    sqlx::query("SELECT id FROM auth_organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .execute(tx)
        .await?;
    Ok(())
}
pub(super) async fn preserve_owner(
    tx: &mut sqlx::PgConnection,
    org: Uuid,
    target: Uuid,
    stays_owner: bool,
) -> Result<(), ApiError> {
    if stays_owner {
        return Ok(());
    }
    let last:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_members WHERE organization_id=$1 AND user_id=$2 AND role='owner') AND (SELECT count(*) FROM auth_members WHERE organization_id=$1 AND role='owner')<=1").bind(org).bind(target).fetch_one(tx).await?;
    if last {
        Err(bad("An organization must retain at least one owner."))
    } else {
        Ok(())
    }
}

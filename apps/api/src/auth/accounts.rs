use super::*;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;
use uuid::Uuid;

static PASSWORD_WORKERS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(4)));
static DUMMY: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(
            b"not-a-real-account-password",
            &SaltString::generate(&mut rand::rngs::OsRng),
        )
        .unwrap()
        .to_string()
});
pub(super) async fn hash(password: String) -> Result<String, ApiError> {
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err(bad(
            "Use a password of at least 12 characters and at most 1024 bytes.",
        ));
    }
    let permit = PASSWORD_WORKERS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| internal())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Argon2::default()
            .hash_password(
                password.as_bytes(),
                &SaltString::generate(&mut rand::rngs::OsRng),
            )
            .map(|h| h.to_string())
            .map_err(|_| internal())
    })
    .await
    .map_err(|_| internal())?
}
pub(super) async fn verify(password: String, hash: Option<String>) -> Result<(), ApiError> {
    if password.len() > 1024 {
        return Err(invalid());
    }
    let permit = PASSWORD_WORKERS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| internal())?;
    let ok = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let value = hash.as_deref().unwrap_or(&DUMMY);
        PasswordHash::new(value).ok().is_some_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .map_err(|_| internal())?;
    if ok { Ok(()) } else { Err(invalid()) }
}
pub(super) fn email(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_lowercase();
    let parts: Vec<_> = value.split('@').collect();
    if value.len() > 254
        || parts.len() != 2
        || parts[0].is_empty()
        || !parts[1].contains('.')
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad("Enter a valid email address."));
    }
    Ok(value)
}
pub(super) fn name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 {
        return Err(bad("Name must contain 1–80 characters."));
    }
    Ok(value.to_owned())
}
pub(super) async fn throttle(pool: &sqlx::PgPool, key: &str, limit: i32) -> Result<(), ApiError> {
    let count:i32=sqlx::query_scalar("INSERT INTO auth_throttles(key,attempts,reset_at) VALUES($1,1,now()+interval '15 minutes') ON CONFLICT(key) DO UPDATE SET attempts=CASE WHEN auth_throttles.reset_at<now() THEN 1 ELSE auth_throttles.attempts+1 END,reset_at=CASE WHEN auth_throttles.reset_at<now() THEN now()+interval '15 minutes' ELSE auth_throttles.reset_at END RETURNING attempts").bind(digest(key)).fetch_one(pool).await?;
    if count > limit {
        Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts. Try again in 15 minutes.",
        ))
    } else {
        Ok(())
    }
}
#[derive(Deserialize)]
pub struct Registration {
    pub name: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub code: String,
}
pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users)")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(serde_json::json!({"setup_required":!exists})))
}
pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Registration>,
) -> Result<Response, ApiError> {
    throttle(&state.pool, "setup", 10).await?;
    let email = email(&input.email)?;
    let name = name(&input.name)?;
    let hash = hash(input.password).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(74781001)")
        .execute(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users)")
        .fetch_one(&mut *tx)
        .await?;
    if exists {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Installation is already configured. Sign in or use an invitation.",
        ));
    }
    let user = Uuid::new_v4();
    let org = Uuid::new_v4();
    sqlx::query("INSERT INTO auth_users(id,name,email,password_hash,system_role) VALUES($1,$2,$3,$4,'admin')").bind(user).bind(name).bind(email).bind(hash).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'My Organization')")
        .bind(org)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'owner')")
        .bind(org)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE projects SET organization_id=$1 WHERE organization_id IS NULL")
        .bind(org)
        .execute(&mut *tx)
        .await?;
    let token = sessions::issue(&mut tx, user, org, &headers).await?;
    log(&mut tx, user, org, "installation.configured").await?;
    tx.commit().await?;
    Ok(sessions::signed_in(&state, &token))
}
#[derive(Deserialize)]
pub struct Credentials {
    email: String,
    password: String,
    #[serde(default)]
    code: String,
}
pub async fn sign_in(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> Result<Response, ApiError> {
    let email = email(&input.email)?;
    throttle(&state.pool, &format!("login:{email}"), 10).await?;
    let found: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id,password_hash,banned FROM auth_users WHERE email=$1")
            .bind(email)
            .fetch_optional(&state.pool)
            .await?;
    verify(input.password, found.as_ref().map(|r| r.1.clone())).await?;
    let (user, old_hash, banned) = found.ok_or_else(invalid)?;
    if banned {
        return Err(invalid());
    }
    let mut tx = state.pool.begin().await?;
    // Serialize against ban/password changes so stale credentials cannot issue a fresh session.
    let current: Option<(String, bool)> =
        sqlx::query_as("SELECT password_hash,banned FROM auth_users WHERE id=$1 FOR UPDATE")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?;
    if current != Some((old_hash, false)) {
        return Err(invalid());
    }
    security::verify_mfa(&mut tx, &state, user, &input.code).await?;
    let org:Uuid=sqlx::query_scalar("SELECT organization_id FROM auth_members WHERE user_id=$1 ORDER BY organization_id LIMIT 1").bind(user).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    let token = sessions::issue(&mut tx, user, org, &headers).await?;
    tx.commit().await?;
    Ok(sessions::signed_in(&state, &token))
}
#[derive(Deserialize)]
pub struct Reauthentication {
    pub password: String,
    #[serde(default)]
    pub code: String,
}
pub(super) async fn check(
    state: &AppState,
    id: &Identity,
    input: Reauthentication,
) -> Result<(), ApiError> {
    throttle(&state.pool, &format!("verify:{}", id.user_id), 10).await?;
    let hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM auth_users WHERE id=$1 AND NOT banned")
            .bind(id.user_id)
            .fetch_optional(&state.pool)
            .await?;
    verify(input.password, hash).await?;
    let mut tx = state.pool.begin().await?;
    security::verify_mfa(&mut tx, state, id.user_id, &input.code).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn reauthenticate(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Reauthentication>,
) -> Result<StatusCode, ApiError> {
    check(&state, &id, input).await?;
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now() WHERE id=$1")
        .bind(id.session_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct ChangePassword {
    #[serde(flatten)]
    credentials: Reauthentication,
    new_password: String,
}
pub async fn change_password(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<ChangePassword>,
) -> Result<Response, ApiError> {
    check(&state, &id, input.credentials).await?;
    let hash = hash(input.new_password).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE auth_users SET password_hash=$1 WHERE id=$2")
        .bind(hash)
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_password_resets WHERE user_id=$1")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, id.user_id, id.organization_id, "password.changed").await?;
    tx.commit().await?;
    Ok((
        [(axum::http::header::SET_COOKIE, state.auth.cookie("", 0))],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
use axum::response::IntoResponse;
pub async fn users(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    id.admin()?;
    let rows: Vec<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "SELECT id,name,email,system_role,banned FROM auth_users ORDER BY created_at LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(|(id,name,email,role,banned)|serde_json::json!({"id":id,"name":name,"email":email,"system_role":role,"banned":banned})).collect()))
}
#[derive(Deserialize)]
pub struct Ban {
    banned: bool,
}
pub async fn ban(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
    Json(input): Json<Ban>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    id.fresh()?;
    if target == id.user_id {
        return Err(bad("You cannot ban your own account."));
    }
    let mut tx = state.pool.begin().await?;
    // System administrators are not bannable through this endpoint; preserves installation recovery.
    let count = sqlx::query("UPDATE auth_users SET banned=$1 WHERE id=$2 AND system_role<>'admin' AND (NOT $1 OR NOT EXISTS(SELECT 1 FROM auth_members WHERE user_id=$2 AND role='owner'))")
        .bind(input.banned)
        .bind(target)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if count == 0 {
        return Err(forbidden());
    }
    if input.banned {
        sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1")
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        if input.banned {
            "admin.user_banned"
        } else {
            "admin.user_unbanned"
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn revoke_user_sessions(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    id.fresh()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1")
        .bind(target)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "admin.sessions_revoked",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn issue_reset(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    id.admin()?;
    id.fresh()?;
    let mut tx = state.pool.begin().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users WHERE id=$1 AND NOT banned)")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
    if !exists {
        return Err(forbidden());
    }
    let token = token();
    sqlx::query("INSERT INTO auth_password_resets(user_id,token_hash) VALUES($1,$2) ON CONFLICT(user_id) DO UPDATE SET token_hash=$2,expires_at=now()+interval '15 minutes'").bind(target).bind(digest(&token)).execute(&mut *tx).await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "admin.password_reset_issued",
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"token":token,"expires_in_seconds":900}),
    ))
}
#[derive(Deserialize)]
pub struct ResetPassword {
    token: String,
    new_password: String,
    #[serde(default)]
    code: String,
}
pub async fn reset_password(
    State(state): State<AppState>,
    Json(input): Json<ResetPassword>,
) -> Result<StatusCode, ApiError> {
    throttle(&state.pool, &format!("reset:{}", digest(&input.token)), 10).await?;
    let hash = hash(input.new_password).await?;
    let mut tx = state.pool.begin().await?;
    let user:Option<Uuid>=sqlx::query_scalar("DELETE FROM auth_password_resets WHERE token_hash=$1 AND expires_at>now() RETURNING user_id").bind(digest(&input.token)).fetch_optional(&mut *tx).await?;
    let user = user.ok_or_else(invalid)?;
    let banned: bool = sqlx::query_scalar("SELECT banned FROM auth_users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    if banned {
        return Err(invalid());
    }
    security::verify_mfa(&mut tx, &state, user, &input.code).await?;
    sqlx::query("UPDATE auth_users SET password_hash=$1 WHERE id=$2")
        .bind(hash)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    // Emit the event in every organization affected by the account credential change.
    sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action) SELECT gen_random_uuid(),user_id,organization_id,'password.reset' FROM auth_members WHERE user_id=$1").bind(user).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

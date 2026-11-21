//! Password-first MFA login. A pre-authentication cookie never authorizes API access.
use super::*;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

fn cookie_name(state: &AppState) -> String {
    format!("{}_mfa", state.auth.cookie_name)
}
fn cookie(state: &AppState, value: &str, age: i64) -> String {
    // Preserve host-only Path=/ and __Host-/Secure rules of the session cookie.
    state
        .auth
        .cookie(value, age)
        .replacen(&state.auth.cookie_name, &cookie_name(state), 1)
}
pub(super) fn clear(state: &AppState, response: &mut Response) {
    response
        .headers_mut()
        .append(header::SET_COOKIE, cookie(state, "", 0).parse().unwrap());
}
fn proof(headers: &HeaderMap, state: &AppState) -> Option<String> {
    sessions::named_cookie_token(headers, &cookie_name(state))
}
fn expired() -> ApiError {
    ApiError(StatusCode::GONE, "Verification expired. Sign in again.")
}
pub(super) async fn begin(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    user: Uuid,
    password_hash: &str,
    factor: &[u8],
) -> Result<Response, ApiError> {
    let value = token();
    sqlx::query("INSERT INTO auth_login_challenges(token_hash,user_id,credential_hash,factor_hash) VALUES($1,$2,$3,$4)")
        .bind(digest(&value)).bind(user).bind(digest(password_hash)).bind(digest(&hex::encode(factor)))
        .execute(tx).await?;
    let mut response =
        Json(serde_json::json!({"two_factor_required":true,"expires_in_seconds":300}))
            .into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie(state, &value, 300).parse().unwrap(),
    );
    // An old browser session must not make the intermediate step look authenticated.
    response.headers_mut().append(
        header::SET_COOKIE,
        state.auth.cookie("", 0).parse().unwrap(),
    );
    Ok(response)
}
#[derive(Deserialize)]
pub struct Verification {
    code: String,
}
pub async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Verification>,
) -> Result<Response, ApiError> {
    let hash = digest(&proof(&headers, &state).ok_or_else(expired)?);
    // Apply the account-wide limit before taking a connection/row lock. Otherwise a
    // burst of submissions can exhaust the pool while waiting for throttle connections.
    let user: Uuid =
        sqlx::query_scalar("SELECT user_id FROM auth_login_challenges WHERE token_hash=$1")
            .bind(&hash)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(expired)?;
    accounts::throttle(&state.pool, &format!("mfa-login:{user}"), 10).await?;
    let mut tx = state.pool.begin().await?;
    // Lock the proof first so concurrent submissions can consume it only once.
    let challenge: Option<(Uuid, String, String, i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT user_id,credential_hash,factor_hash,attempts,expires_at FROM auth_login_challenges WHERE token_hash=$1 FOR UPDATE")
        .bind(&hash).fetch_optional(&mut *tx).await?;
    let (user, credential, factor, attempts, deadline) = challenge.ok_or_else(expired)?;
    if deadline <= Utc::now() || attempts >= 5 {
        return Err(expired());
    }

    let current: Option<(String, bool)> =
        sqlx::query_as("SELECT password_hash,auth_user_banned(banned,ban_expires_at) FROM auth_users WHERE id=$1 FOR UPDATE")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?;
    let enabled: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT secret FROM auth_mfa WHERE user_id=$1 AND enabled")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?;
    // Changing a password, disabling/re-enrolling MFA or banning invalidates pending proofs.
    if !current.is_some_and(|(password, banned)| !banned && digest(&password) == credential)
        || !enabled.is_some_and(|secret| digest(&hex::encode(secret)) == factor)
    {
        return Err(expired());
    }
    sqlx::query("UPDATE auth_login_challenges SET attempts=attempts+1 WHERE token_hash=$1")
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    if let Err(error) = security::verify_mfa(&mut tx, &state, user, &input.code).await {
        // Commit failed attempts, rather than rolling them back with an invalid code.
        tx.commit().await?;
        return Err(error);
    }
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM auth_members WHERE user_id=$1 ORDER BY organization_id LIMIT 1")
        .bind(user).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    sqlx::query("DELETE FROM auth_login_challenges WHERE token_hash=$1")
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    let session = sessions::issue(&mut tx, user, org, &headers).await?;
    tx.commit().await?;
    Ok(sessions::signed_in(&state, &session))
}
pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(value) = proof(&headers, &state) {
        sqlx::query("DELETE FROM auth_login_challenges WHERE token_hash=$1")
            .bind(digest(&value))
            .execute(&state.pool)
            .await?;
    }
    Ok((
        [(header::SET_COOKIE, cookie(&state, "", 0))],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

use super::*;
use axum::{Json, extract::State, http::StatusCode};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use totp_rs::{Algorithm, TOTP};
use uuid::Uuid;

pub(super) fn equal(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}
fn encrypt(state: &AppState, user: Uuid, plain: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new((&state.auth.key).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: user.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    Ok([nonce.to_vec(), ciphertext].concat())
}
fn decrypt(state: &AppState, user: Uuid, data: &[u8]) -> Result<Vec<u8>, ApiError> {
    if data.len() < 40 {
        return Err(internal());
    }
    XChaCha20Poly1305::new((&state.auth.key).into())
        .decrypt(
            XNonce::from_slice(&data[..24]),
            Payload {
                msg: &data[24..],
                aad: user.as_bytes(),
            },
        )
        .map_err(|_| internal())
}
fn totp(secret: Vec<u8>, label: String) -> Result<TOTP, ApiError> {
    TOTP::new(
        Algorithm::SHA1,
        6,
        0,
        30,
        secret,
        Some("Ferrix".into()),
        label.replace(':', "_"),
    )
    .map_err(|_| internal())
}
fn matched_step(totp: &TOTP, code: &str, last: i64) -> Option<i64> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let step = chrono::Utc::now().timestamp() / 30;
    [step, step - 1, step + 1]
        .into_iter()
        .find(|s| *s > last && equal(totp.generate((*s as u64) * 30).as_bytes(), code.as_bytes()))
}
pub(super) async fn verify_mfa(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    user: Uuid,
    code: &str,
) -> Result<(), ApiError> {
    let row: Option<(Vec<u8>, i64)> = sqlx::query_as(
        "SELECT secret,last_step FROM auth_mfa WHERE user_id=$1 AND enabled FOR UPDATE",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((secret, last)) = row {
        let authenticator = totp(decrypt(state, user, &secret)?, "account".into())?;
        if let Some(step) = matched_step(&authenticator, code, last) {
            sqlx::query("UPDATE auth_mfa SET last_step=$1 WHERE user_id=$2")
                .bind(step)
                .bind(user)
                .execute(&mut *tx)
                .await?;
        } else {
            let deleted =
                sqlx::query("DELETE FROM auth_recovery_codes WHERE user_id=$1 AND code_hash=$2")
                    .bind(user)
                    .bind(digest(code))
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            if deleted != 1 {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
pub async fn status(State(state): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    let enabled: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_mfa WHERE user_id=$1 AND enabled)")
            .bind(id.user_id)
            .fetch_one(&state.pool)
            .await?;
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_recovery_codes WHERE user_id=$1")
            .bind(id.user_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(
        json!({"two_factor_enabled":enabled,"recovery_codes_remaining":remaining,"fresh_until":id.authenticated_at+chrono::Duration::minutes(15)}),
    ))
}
pub async fn enroll(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<accounts::Reauthentication>,
) -> Result<Json<Value>, ApiError> {
    accounts::check(&state, &id, input).await?;
    let mut secret = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let authenticator = totp(secret.to_vec(), id.email.clone())?;
    let data = encrypt(&state, id.user_id, &secret)?;
    let n=sqlx::query("INSERT INTO auth_mfa(user_id,secret) VALUES($1,$2) ON CONFLICT(user_id) DO UPDATE SET secret=$2,last_step=-1,pending_expires_at=now()+interval '10 minutes' WHERE NOT auth_mfa.enabled").bind(id.user_id).bind(data).execute(&state.pool).await?.rows_affected();
    if n == 0 {
        return Err(bad("Two-factor authentication is already enabled."));
    }
    // The enrollment password check is a fresh verification for this browser too.
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now() WHERE id=$1")
        .bind(id.session_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(
        json!({"secret":authenticator.get_secret_base32(),"otpauth_url":authenticator.get_url()}),
    ))
}
#[derive(Deserialize)]
pub struct Code {
    code: String,
}
pub async fn confirm(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Code>,
) -> Result<Json<Value>, ApiError> {
    id.fresh()?;
    accounts::throttle(&state.pool, &format!("mfa-confirm:{}", id.user_id), 10).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM auth_users WHERE id=$1 FOR UPDATE")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    let data:Option<Vec<u8>>=sqlx::query_scalar("SELECT secret FROM auth_mfa WHERE user_id=$1 AND NOT enabled AND pending_expires_at>now() FOR UPDATE").bind(id.user_id).fetch_optional(&mut *tx).await?;
    let data = data.ok_or_else(invalid)?;
    let auth = totp(decrypt(&state, id.user_id, &data)?, id.email.clone())?;
    let step = matched_step(&auth, &input.code, -1).ok_or_else(invalid)?;
    sqlx::query("UPDATE auth_mfa SET enabled=true,last_step=$1 WHERE user_id=$2")
        .bind(step)
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_recovery_codes WHERE user_id=$1")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    let codes: Vec<String> = (0..8).map(|_| token()).collect();
    for code in &codes {
        sqlx::query("INSERT INTO auth_recovery_codes(user_id,code_hash) VALUES($1,$2)")
            .bind(id.user_id)
            .bind(digest(code))
            .execute(&mut *tx)
            .await?;
    }
    // All other sessions predate the second factor and must reauthenticate.
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1 AND id<>$2")
        .bind(id.user_id)
        .bind(id.session_id)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, id.user_id, id.organization_id, "mfa.enabled").await?;
    tx.commit().await?;
    Ok(Json(json!({"recovery_codes":codes})))
}
pub async fn disable(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<accounts::Reauthentication>,
) -> Result<StatusCode, ApiError> {
    accounts::check(&state, &id, input).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM auth_users WHERE id=$1 FOR UPDATE")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_mfa WHERE user_id=$1")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_recovery_codes WHERE user_id=$1")
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1 AND id<>$2")
        .bind(id.user_id)
        .bind(id.session_id)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, id.user_id, id.organization_id, "mfa.disabled").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn audit(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows:Vec<(Uuid,Option<Uuid>,String,chrono::DateTime<chrono::Utc>)>=sqlx::query_as("SELECT id,user_id,action,created_at FROM auth_audit WHERE organization_ref=$1 AND (actor_id=$2 OR $3) AND action NOT LIKE 'admin.%' AND action NOT LIKE 'platform.%' ORDER BY created_at DESC,id DESC LIMIT 100").bind(id.organization_id).bind(id.user_id).bind(matches!(id.role.as_str(),"owner"|"admin")).fetch_all(&state.pool).await?;
    Ok(Json(rows.into_iter().map(|(id,user_id,action,created_at)|json!({"id":id,"user_id":user_id,"action":action,"created_at":created_at})).collect()))
}

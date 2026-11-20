pub(crate) mod accounts;
mod organizations;
mod security;
pub(crate) mod sessions;
use crate::{ApiError, AppState};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use rand::RngCore;
pub use sessions::{AuthConfig, Identity};
use sha2::{Digest, Sha256};

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/session", get(sessions::current))
        .route("/sign-out", post(sessions::sign_out))
        .route("/sessions", get(sessions::list))
        .route("/sessions/{id}/revoke", post(sessions::revoke))
        .route("/sessions/revoke-others", post(sessions::revoke_others))
        .route("/reauthenticate", post(accounts::reauthenticate))
        .route("/change-password", post(accounts::change_password))
        .route(
            "/organizations",
            get(organizations::list).post(organizations::create),
        )
        .route("/organizations/switch", post(organizations::switch))
        .route("/members", get(organizations::members))
        .route("/members/{id}/role", post(organizations::role))
        .route("/members/{id}/remove", post(organizations::remove))
        .route(
            "/invitations",
            get(organizations::invitations).post(organizations::invite),
        )
        .route(
            "/invitations/{id}/revoke",
            post(organizations::revoke_invite),
        )
        .route("/admin/users", get(accounts::users))
        .route(
            "/admin/users/{id}/password-reset",
            post(accounts::issue_reset),
        )
        .route("/admin/users/{id}/ban", post(accounts::ban))
        .route(
            "/admin/users/{id}/revoke-sessions",
            post(accounts::revoke_user_sessions),
        )
        .route("/security", get(security::status))
        .route("/audit", get(security::audit))
        .route("/mfa/enroll", post(security::enroll))
        .route("/mfa/confirm", post(security::confirm))
        .route("/mfa/disable", post(security::disable))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            sessions::protect,
        ))
        .route("/setup", get(accounts::setup_status).post(accounts::setup))
        .route("/sign-in", post(accounts::sign_in))
        .route("/reset-password", post(accounts::reset_password))
        .route("/accept-invitation", post(organizations::accept))
        .layer(DefaultBodyLimit::max(16 * 1024))
}

pub(super) fn token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub(super) fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub(super) fn invalid() -> ApiError {
    ApiError(
        axum::http::StatusCode::UNAUTHORIZED,
        "Invalid credentials or verification code.",
    )
}
pub(super) fn forbidden() -> ApiError {
    ApiError(
        axum::http::StatusCode::FORBIDDEN,
        "You do not have permission for this action.",
    )
}
pub(super) fn bad(message: &'static str) -> ApiError {
    ApiError(axum::http::StatusCode::UNPROCESSABLE_ENTITY, message)
}
pub(super) fn internal() -> ApiError {
    ApiError(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "Authentication operation failed.",
    )
}
pub(super) async fn log(
    tx: &mut sqlx::PgConnection,
    user: uuid::Uuid,
    org: uuid::Uuid,
    action: &str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action) VALUES($1,$2,$3,$4)")
        .bind(uuid::Uuid::new_v4())
        .bind(user)
        .bind(org)
        .bind(action)
        .execute(tx)
        .await?;
    Ok(())
}

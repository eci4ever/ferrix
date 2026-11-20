use super::*;
use axum::{
    Json,
    extract::Request,
    extract::{FromRequestParts, Path, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone)]
pub struct AuthConfig {
    pub origin: String,
    pub secure: bool,
    pub cookie_name: String,
    pub key: [u8; 32],
}
impl AuthConfig {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let origin = std::env::var("FERRIX_AUTH_ORIGIN")?;
        let url = url::Url::parse(&origin)?;
        let secure = url.scheme() == "https";
        if url.origin().ascii_serialization() != origin
            || (!secure
                && !(url.scheme() == "http"
                    && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))))
        {
            return Err(
                "Auth origin must be an exact HTTPS origin (HTTP allowed on loopback only)".into(),
            );
        }
        let key: [u8; 32] = hex::decode(std::env::var("FERRIX_AUTH_KEY")?)?
            .try_into()
            .map_err(|_| "FERRIX_AUTH_KEY must contain 32 hex-encoded bytes")?;
        let port = url.port_or_known_default().ok_or("Missing origin port")?;
        let cookie_name = format!(
            "{}ferrix_{port}_session",
            if secure { "__Host-" } else { "" }
        );
        Ok(Self {
            origin,
            secure,
            cookie_name,
            key,
        })
    }
    pub fn cookie(&self, value: &str, age: i64) -> String {
        format!(
            "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{}",
            self.cookie_name,
            if self.secure { "; Secure" } else { "" }
        )
    }
}

#[derive(Clone, Serialize, FromRow)]
pub struct Identity {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub system_role: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub role: String,
    pub authenticated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
impl Identity {
    pub(crate) fn manager(&self) -> Result<(), ApiError> {
        if matches!(self.role.as_str(), "owner" | "admin") {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
    pub(crate) fn admin(&self) -> Result<(), ApiError> {
        if self.system_role == "admin" {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
    pub(super) fn fresh(&self) -> Result<(), ApiError> {
        if self.authenticated_at > Utc::now() - chrono::Duration::minutes(15) {
            Ok(())
        } else {
            Err(ApiError(
                StatusCode::UNAUTHORIZED,
                "Please verify your password again before this action.",
            ))
        }
    }
}
impl FromRequestParts<AppState> for Identity {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, ApiError> {
        parts.extensions.get::<Self>().cloned().ok_or_else(invalid)
    }
}
pub(super) fn cookie_token(headers: &HeaderMap, config: &AuthConfig) -> Option<String> {
    let mut tokens = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|v| v.trim().split_once('='))
        .filter(|(k, _)| *k == config.cookie_name)
        .map(|(_, v)| v);
    let t = tokens.next()?;
    if tokens.next().is_some() || t.len() != 64 || !t.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(t.to_owned())
}
pub(super) async fn resolve(state: &AppState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    let t = cookie_token(headers, &state.auth).ok_or_else(invalid)?;
    sqlx::query_as::<_,Identity>("SELECT s.id session_id,u.id user_id,u.name,u.email,u.system_role,s.active_organization_id organization_id,o.name organization_name,m.role,s.authenticated_at,s.expires_at FROM auth_sessions s JOIN auth_users u ON u.id=s.user_id JOIN auth_members m ON m.user_id=u.id AND m.organization_id=s.active_organization_id JOIN auth_organizations o ON o.id=m.organization_id WHERE s.token_hash=$1 AND s.expires_at>now() AND s.absolute_expires_at>now() AND NOT u.banned")
 .bind(digest(&t)).fetch_optional(&state.pool).await?.ok_or_else(invalid)
}
// Background clock polling authenticates without extending session inactivity expiry.
pub async fn identify(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    match resolve(&state, req.headers()).await {
        Ok(id) => {
            req.extensions_mut().insert(id);
            next.run(req).await
        }
        Err(error) => error.into_response(),
    }
}

pub async fn protect(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let mut id = match resolve(&state, req.headers()).await {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let renewed: Result<Option<DateTime<Utc>>, sqlx::Error> = sqlx::query_scalar("UPDATE auth_sessions SET expires_at=LEAST(now()+interval '3 days',absolute_expires_at),updated_at=now() WHERE id=$1 AND updated_at<now()-interval '1 day' AND expires_at>now() AND absolute_expires_at>now() RETURNING expires_at").bind(id.session_id).fetch_optional(&state.pool).await;
    let cookie = match renewed {
        Ok(Some(expires)) => {
            id.expires_at = expires;
            cookie_token(req.headers(), &state.auth).map(|token| {
                state
                    .auth
                    .cookie(&token, (expires - Utc::now()).num_seconds().max(0))
            })
        }
        Ok(None) => None,
        Err(e) => return ApiError::from(e).into_response(),
    };
    req.extensions_mut().insert(id);
    let mut response = next.run(req).await;
    // Preserve explicit cookie clearing from logout/password change.
    if !response.headers().contains_key(header::SET_COOKIE)
        && let Some(cookie) = cookie
    {
        response
            .headers_mut()
            .insert(header::SET_COOKIE, cookie.parse().unwrap());
    }
    response
}
// Origin is required for every unsafe API call, including unauthenticated login/setup.
// SameSite and JSON requests provide additional browser-side defenses; no CORS is enabled.
pub async fn guard(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    req.headers_mut().remove("x-ferrix-peer");
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|p| p.0.ip().to_string())
        .unwrap_or_else(|| "local".into());
    if let Ok(value) = peer.parse() {
        req.headers_mut().insert("x-ferrix-peer", value);
    }
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        != Some(state.auth.origin.as_str())
    {
        return forbidden().into_response();
    }
    if req.uri().path().starts_with("/api/v1/auth")
        && req.method() != axum::http::Method::GET
        && let Err(e) = accounts::throttle(&state.pool, &format!("peer:{peer}"), 120).await
    {
        return e.into_response();
    }
    let mut response = next.run(req).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response
}
pub(super) async fn issue(
    tx: &mut sqlx::PgConnection,
    user: Uuid,
    org: Uuid,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let t = token();
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(256)
        .collect::<String>();
    // Client forwarded headers are intentionally not trusted.
    sqlx::query("INSERT INTO auth_sessions(id,token_hash,user_id,active_organization_id,user_agent,ip_address) VALUES($1,$2,$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(digest(&t)).bind(user).bind(org).bind(agent).bind(headers.get("x-ferrix-peer").and_then(|v|v.to_str().ok()).unwrap_or("")).execute(&mut *tx).await?;
    log(tx, user, org, "session.created").await?;
    Ok(t)
}
pub(super) fn signed_in(state: &AppState, t: &str) -> Response {
    (
        [(header::SET_COOKIE, state.auth.cookie(t, 259200))],
        Json(serde_json::json!({"ok":true})),
    )
        .into_response()
}
pub async fn current(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let mut id = resolve(&state, &headers).await?;
    let expires:DateTime<Utc>=sqlx::query_scalar("UPDATE auth_sessions SET expires_at=CASE WHEN updated_at<now()-interval '1 day' THEN LEAST(now()+interval '3 days',absolute_expires_at) ELSE expires_at END,updated_at=CASE WHEN updated_at<now()-interval '1 day' THEN now() ELSE updated_at END WHERE id=$1 RETURNING expires_at").bind(id.session_id).fetch_one(&state.pool).await?;
    id.expires_at = expires;
    let t = cookie_token(&headers, &state.auth).ok_or_else(invalid)?;
    let age = (expires - Utc::now()).num_seconds().max(0);
    Ok(([(header::SET_COOKIE, state.auth.cookie(&t, age))], Json(id)).into_response())
}
pub async fn sign_out(State(state): State<AppState>, id: Identity) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM auth_sessions WHERE id=$1")
        .bind(id.session_id)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "session.signed_out",
    )
    .await?;
    tx.commit().await?;
    Ok((
        [(header::SET_COOKIE, state.auth.cookie("", 0))],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
#[derive(Serialize, FromRow)]
pub struct Session {
    id: Uuid,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    user_agent: String,
    ip_address: String,
    current: bool,
}
pub async fn list(
    State(state): State<AppState>,
    id: Identity,
) -> Result<Json<Vec<Session>>, ApiError> {
    Ok(Json(sqlx::query_as("SELECT id,created_at,expires_at,user_agent,ip_address,(id=$2) current FROM auth_sessions WHERE user_id=$1 AND expires_at>now() AND absolute_expires_at>now() ORDER BY created_at DESC").bind(id.user_id).bind(id.session_id).fetch_all(&state.pool).await?))
}
pub async fn revoke(
    State(state): State<AppState>,
    id: Identity,
    Path(target): Path<Uuid>,
) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    let deleted = sqlx::query("DELETE FROM auth_sessions WHERE id=$1 AND user_id=$2")
        .bind(target)
        .bind(id.user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if deleted > 0 {
        log(&mut tx, id.user_id, id.organization_id, "session.revoked").await?;
    }
    tx.commit().await?;
    if target == id.session_id {
        Ok((
            [(header::SET_COOKIE, state.auth.cookie("", 0))],
            StatusCode::NO_CONTENT,
        )
            .into_response())
    } else {
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}
pub async fn revoke_others(
    State(state): State<AppState>,
    id: Identity,
) -> Result<StatusCode, ApiError> {
    id.fresh()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id=$1 AND id<>$2")
        .bind(id.user_id)
        .bind(id.session_id)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "session.others_revoked",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

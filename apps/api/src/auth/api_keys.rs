//! User-owned, organization-bound bearer credentials for non-browser deployment agents.
//! Browser sessions remain the only way to create/revoke credentials or administer accounts.
use super::*;
use axum::{
    Json,
    extract::{MatchedPath, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

pub const SCOPES: &[&str] = &["read", "manage", "deploy", "delete", "secrets:read"];
const RATE_LIMIT: i32 = 120;
#[derive(Clone)]
pub(crate) struct KeyAccess {
    pub id: Uuid,
    pub scopes: Vec<String>,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    pub name: String,
}
#[derive(Serialize, FromRow)]
struct KeyInfo {
    id: Uuid,
    project_id: Option<Uuid>,
    project_name: Option<String>,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
}
pub async fn list(State(state): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    let rows:Vec<KeyInfo>=sqlx::query_as("SELECT k.id,k.name,k.prefix,k.scopes,k.created_at,k.expires_at,k.revoked_at,k.last_used_at,k.project_id,p.name AS project_name FROM auth_api_keys k LEFT JOIN projects p ON p.id=k.project_id WHERE k.user_id=$1 AND k.organization_id=$2 ORDER BY (k.revoked_at IS NULL AND k.expires_at>now()) DESC,k.created_at DESC,k.id LIMIT 100")
        .bind(id.user_id).bind(id.organization_id).fetch_all(&state.pool).await?;
    let projects: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id,name FROM projects WHERE organization_id=$1 ORDER BY name,id")
            .bind(id.organization_id)
            .fetch_all(&state.pool)
            .await?;
    let projects: Vec<Value> = projects
        .into_iter()
        .map(|(id, name)| json!({"id":id,"name":name}))
        .collect();
    Ok(Json(
        json!({"projects":projects,"keys":rows,"scopes":SCOPES,"rate_limit_per_minute":RATE_LIMIT}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    name: String,
    scopes: Vec<String>,
    expires_in_days: u32,
    project_id: Option<Uuid>,
}
pub async fn create(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Create>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let name = accounts::name(&input.name)?;
    let mut scopes = input.scopes;
    scopes.sort();
    scopes.dedup();
    if scopes.is_empty()
        || !scopes.iter().any(|s| s == "read")
        || scopes.iter().any(|s| !SCOPES.contains(&s.as_str()))
        || !(1..=365).contains(&input.expires_in_days)
    {
        return Err(bad(
            "Include read, choose supported scopes and an expiry of 1–365 days.",
        ));
    }
    let value = format!("frx_{}", token());
    let key = Uuid::new_v4();
    let prefix = &value[..12];
    let expiry = Utc::now() + chrono::Duration::days(input.expires_in_days.into());
    let mut tx = state.pool.begin().await?;
    let role = management::authorize(&mut tx, &id, "member").await?;
    if role == "member" && scopes.iter().any(|s| s != "read") {
        return Err(forbidden());
    }
    if let Some(project) = input.project_id {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1 AND organization_id=$2)",
        )
        .bind(project)
        .bind(id.organization_id)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                "Project not found in this organization.",
            ));
        }
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM auth_api_keys WHERE user_id=$1 AND revoked_at IS NULL AND expires_at>now()")
        .bind(id.user_id).fetch_one(&mut *tx).await?;
    if count >= 20 {
        return Err(bad(
            "Revoke an existing key before creating more than 20 active keys.",
        ));
    }
    sqlx::query("INSERT INTO auth_api_keys(id,user_id,organization_id,name,key_hash,prefix,scopes,expires_at,project_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(key).bind(id.user_id).bind(id.organization_id).bind(name).bind(digest(&value)).bind(prefix).bind(scopes).bind(expiry).bind(input.project_id).execute(&mut *tx).await?;
    log(&mut tx, id.user_id, id.organization_id, "api_key.created").await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":key,"key":value,"expires_at":expiry})),
    ))
}
pub async fn revoke(
    State(state): State<AppState>,
    id: Identity,
    Path(key): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "member").await?;
    let n=sqlx::query("UPDATE auth_api_keys SET revoked_at=COALESCE(revoked_at,now()) WHERE id=$1 AND user_id=$2 AND organization_id=$3")
        .bind(key).bind(id.user_id).bind(id.organization_id).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "API key not found."));
    }
    log(&mut tx, id.user_id, id.organization_id, "api_key.revoked").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn admin_revoke(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    sqlx::query("UPDATE auth_api_keys SET revoked_at=COALESCE(revoked_at,now()) WHERE user_id=$1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        "admin.api_keys_revoked",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn usage(
    State(state): State<AppState>,
    id: Identity,
    Path(key): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let owns:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_api_keys WHERE id=$1 AND user_id=$2 AND organization_id=$3)").bind(key).bind(id.user_id).bind(id.organization_id).fetch_one(&state.pool).await?;
    if !owns {
        return Err(ApiError(StatusCode::NOT_FOUND, "API key not found."));
    }
    let rows:Vec<Usage>=sqlx::query_as("SELECT id,method,route,resource_id,status,ip_address,created_at FROM auth_api_key_requests WHERE key_id=$1 AND expires_at>now() ORDER BY created_at DESC,id LIMIT 100")
        .bind(key).fetch_all(&state.pool).await?;
    Ok(Json(json!({"requests":rows})))
}
#[derive(Serialize, FromRow)]
struct Usage {
    id: Uuid,
    method: String,
    route: String,
    resource_id: Option<Uuid>,
    status: i16,
    ip_address: String,
    created_at: DateTime<Utc>,
}

// This allowlist is deliberately independent of platform-admin status. New routes
// must be classified explicitly; a future endpoint never inherits key access.
fn scope(method: &str, route: &str) -> Option<&'static str> {
    let route = route.strip_prefix("/api/v1")?;
    match (method, route) {
        (
            "GET",
            "/agent"
            | "/agent/docs"
            | "/projects"
            | "/projects/service-status"
            | "/home-summary"
            | "/status"
            | "/server-time"
            | "/projects/{id}/environments"
            | "/projects/{id}/quota"
            | "/environments/{id}/applications"
            | "/environments/{id}/services"
            | "/environments/{id}/service-status"
            | "/applications/{id}"
            | "/applications/{id}/runtime"
            | "/applications/{id}/logs"
            | "/applications/{id}/deployments"
            | "/services/{id}"
            | "/services/{id}/runtime"
            | "/services/{id}/metrics"
            | "/services/{id}/history"
            | "/services/{id}/logs/history"
            | "/services/{id}/logs"
            | "/services/{id}/deployments"
            | "/services/{id}/domains"
            | "/services/{id}/backups",
        ) => Some("read"),
        (
            "GET",
            "/applications/{id}/configuration"
            | "/services/{id}/configuration"
            | "/services/{id}/environment"
            | "/services/{id}/source"
            | "/services/{id}/domains/{domain_id}/tunnel-config",
        ) => Some("secrets:read"),
        (
            "POST",
            "/projects"
            | "/projects/{id}/environments"
            | "/environments/{id}/applications"
            | "/environments/{id}/services"
            | "/applications/{id}/configuration"
            | "/services/{id}/configuration"
            | "/services/{id}/environment"
            | "/services/{id}/domains"
            | "/services/{id}/domains/{domain_id}/check"
            | "/services/{id}/domains/{domain_id}/preferred"
            | "/services/{id}/source"
            | "/services/{id}/source/upload",
        ) => Some("manage"),
        (
            "POST",
            "/applications/{id}/deploy"
            | "/services/{id}/deploy"
            | "/services/{id}/control"
            | "/services/{id}/rollback",
        ) => Some("deploy"),
        ("POST", "/services/{id}/backups") => Some("deploy"),
        ("POST", "/services/{id}/backups/{backup_id}/restore") => Some("manage+deploy"),
        ("DELETE", "/projects/{id}" | "/services/{id}" | "/services/{id}/domains/{domain_id}") => {
            Some("delete")
        }
        _ => None,
    }
}
fn unauthorized() -> ApiError {
    ApiError(
        StatusCode::UNAUTHORIZED,
        "Invalid, expired or revoked API key.",
    )
}
fn credential(headers: &HeaderMap, state: &AppState) -> Result<String, ApiError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)?;
    if values.next().is_some() {
        return Err(unauthorized());
    }
    let (scheme, value) = value.split_once(' ').ok_or_else(unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || value.len() != 68
        || !value.starts_with("frx_")
        || !value.as_bytes()[4..].iter().all(|b| b.is_ascii_hexdigit())
    {
        return Err(unauthorized());
    }
    let cookie = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|v| v.trim().split_once('='))
        .any(|(name, _)| name == state.auth.cookie_name);
    if cookie {
        return Err(bad("Use an API key without a browser session cookie."));
    }
    Ok(value.to_owned())
}
#[derive(FromRow)]
struct Principal {
    key_id: Uuid,
    key_name: String,
    project_id: Option<Uuid>,
    project_name: Option<String>,
    scopes: Vec<String>,
    user_id: Uuid,
    name: String,
    email: String,
    system_role: String,
    organization_id: Uuid,
    organization_name: String,
    role: String,
    expires_at: DateTime<Utc>,
}
async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<Principal, ApiError> {
    let peer = headers
        .get("x-ferrix-peer")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("local");
    let value = match credential(headers, state) {
        Ok(value) => value,
        Err(error) => {
            accounts::throttle(&state.pool, &format!("api-key-invalid:{peer}"), 240).await?;
            return Err(error);
        }
    };
    let found=sqlx::query_as("SELECT k.id key_id,k.name key_name,k.project_id,p.name AS project_name,k.scopes,u.id user_id,u.name,u.email,u.system_role,o.id organization_id,o.name organization_name,m.role,k.expires_at FROM auth_api_keys k JOIN auth_users u ON u.id=k.user_id JOIN auth_members m ON m.user_id=u.id AND m.organization_id=k.organization_id JOIN auth_organizations o ON o.id=m.organization_id LEFT JOIN projects p ON p.id=k.project_id AND p.organization_id=k.organization_id WHERE (k.project_id IS NULL OR p.id IS NOT NULL) AND k.key_hash=$1 AND k.revoked_at IS NULL AND k.expires_at>now() AND NOT auth_user_banned(u.banned,u.ban_expires_at)")
        .bind(digest(&value)).fetch_optional(&state.pool).await?;
    if let Some(found) = found {
        Ok(found)
    } else {
        accounts::throttle(&state.pool, &format!("api-key-invalid:{peer}"), 240).await?;
        Err(unauthorized())
    }
}
async fn rate(state: &AppState, key: Uuid) -> Result<i32, ApiError> {
    let count:i32=sqlx::query_scalar("UPDATE auth_api_keys SET rate_count=CASE WHEN rate_window<date_trunc('minute',now()) THEN 1 ELSE LEAST(rate_count+1,122) END,rate_window=date_trunc('minute',now()),last_used_at=now() WHERE id=$1 AND revoked_at IS NULL AND expires_at>now() RETURNING rate_count")
        .bind(key).fetch_optional(&state.pool).await?.ok_or_else(unauthorized)?;
    Ok(count)
}
async fn project_access(
    state: &AppState,
    project: Option<Uuid>,
    method: &str,
    route: &str,
    resource: Option<Uuid>,
) -> Result<bool, ApiError> {
    let Some(project) = project else {
        return Ok(true);
    };
    let route = route.strip_prefix("/api/v1").unwrap_or("");
    match (method, route) {
        ("GET", "/agent" | "/agent/docs" | "/projects" | "/projects/service-status" | "/home-summary" | "/status" | "/server-time") => Ok(true),
        (_, "/projects/{id}" | "/projects/{id}/environments" | "/projects/{id}/quota") => Ok(resource == Some(project)),
        (_, "/environments/{id}/applications" | "/environments/{id}/services" | "/environments/{id}/service-status") => {
            Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM environments WHERE id=$1 AND project_id=$2)").bind(resource).bind(project).fetch_one(&state.pool).await?)
        },
        (_, path) if path == "/services/{id}" || path.starts_with("/services/{id}/") || path == "/applications/{id}" || path.starts_with("/applications/{id}/") => {
            Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id WHERE a.id=$1 AND e.project_id=$2)").bind(resource).bind(project).fetch_one(&state.pool).await?)
        },
        _ => Ok(false),
    }
}
pub(crate) async fn run(state: &AppState, mut req: Request, next: Next) -> Response {
    let principal = match authenticate(state, req.headers()).await {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let method = req.method().as_str().to_owned();
    let resource = req
        .uri()
        .path()
        .split('/')
        .find_map(|part| Uuid::parse_str(part).ok());
    let peer = req
        .headers()
        .get("x-ferrix-peer")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
        .to_owned();
    let result = rate(state, principal.key_id).await;
    let record = result.as_ref().is_ok_and(|count| *count <= RATE_LIMIT + 1);
    let allowed = scope(&method, &route).is_some_and(|required| {
        required
            .split('+')
            .all(|scope| principal.scopes.iter().any(|value| value == scope))
            && (required == "read" || matches!(principal.role.as_str(), "owner" | "admin"))
    });
    let project_access = if allowed {
        project_access(state, principal.project_id, &method, &route, resource).await
    } else {
        Ok(false)
    };
    let response = match result {
        Err(error) => error.into_response(),
        Ok(count) if count > RATE_LIMIT => (
            [(header::RETRY_AFTER, "60")],
            ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "API key rate limit exceeded. Retry after one minute.",
            ),
        )
            .into_response(),
        Ok(_) if !allowed => forbidden().into_response(),
        Ok(_) if matches!(project_access, Ok(false)) => ApiError(
            StatusCode::NOT_FOUND,
            "Resource is outside this key's project access.",
        )
        .into_response(),
        Ok(_) if project_access.is_err() => project_access.unwrap_err().into_response(),
        Ok(_) => {
            req.extensions_mut().insert(Identity {
                session_id: Uuid::nil(),
                user_id: principal.user_id,
                name: principal.name,
                email: principal.email,
                system_role: principal.system_role,
                organization_id: principal.organization_id,
                organization_name: principal.organization_name,
                role: principal.role,
                authenticated_at: DateTime::UNIX_EPOCH,
                expires_at: principal.expires_at,
            });
            req.extensions_mut().insert(KeyAccess {
                id: principal.key_id,
                scopes: principal.scopes,
                project_id: principal.project_id,
                project_name: principal.project_name,
                name: principal.key_name,
            });
            next.run(req).await
        }
    };
    // Only matched route templates and UUIDs are retained: no bodies, queries, keys or headers.
    if record && let Err(error)=sqlx::query("INSERT INTO auth_api_key_requests(id,key_id,user_id,organization_id,method,route,resource_id,status,ip_address) VALUES($1,(SELECT id FROM auth_api_keys WHERE id=$2),$3,$4,$5,$6,$7,$8,$9)")
        .bind(Uuid::new_v4()).bind(principal.key_id).bind(principal.user_id).bind(principal.organization_id).bind(method).bind(route).bind(resource).bind(i16::try_from(response.status().as_u16()).unwrap_or(500)).bind(peer).execute(&state.pool).await{
        tracing::warn!(%error,"Could not record API key request metadata");
    }
    response
}
pub async fn agent(
    id: Identity,
    key: Option<axum::Extension<KeyAccess>>,
) -> Result<Json<Value>, ApiError> {
    let key = key.ok_or_else(forbidden)?.0;
    let effective: Vec<_> = key
        .scopes
        .iter()
        .filter(|scope| scope.as_str() == "read" || matches!(id.role.as_str(), "owner" | "admin"))
        .collect();
    let workflow = if key.project_id.is_some() {
        vec![
            "GET /api/v1/projects",
            "GET /api/v1/projects/{id}/environments",
            "POST /api/v1/environments/{id}/services",
            "POST /api/v1/services/{id}/deploy",
            "GET /api/v1/services/{id}/deployments",
            "GET /api/v1/services/{id}/runtime",
        ]
    } else {
        vec![
            "GET /api/v1/projects",
            "POST /api/v1/projects",
            "GET /api/v1/projects/{id}/environments",
            "POST /api/v1/environments/{id}/services",
            "POST /api/v1/services/{id}/deploy",
            "GET /api/v1/services/{id}/deployments",
            "GET /api/v1/services/{id}/runtime",
        ]
    };
    Ok(Json(
        json!({"project_id":key.project_id,"project_name":key.project_name,"api_version":"v1","user_id":id.user_id,"organization_id":id.organization_id,"organization_name":id.organization_name,"organization_role":id.role,"key_id":key.id,"scopes":effective,"granted_scopes":key.scopes,"rate_limit_per_minute":RATE_LIMIT,"documentation":"/api/v1/agent/docs","workflow":workflow,"constraints":["Scopes never exceed the current organization role","Account and organization administration require a browser session","Configuration reads require secrets:read; deployment and application logs may contain sensitive application output"]}),
    ))
}

pub async fn documentation(_id: Identity) -> Response {
    (
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        include_str!("../../../../docs/api-keys.md"),
    )
        .into_response()
}

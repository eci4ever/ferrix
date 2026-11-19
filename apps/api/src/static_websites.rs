use crate::{ApiError, AppState, auth::Identity};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, Uri, header},
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const MAX_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn routes(state: AppState) -> Router<AppState> {
    let reads = Router::new()
        .route("/environments/{id}/static-websites", get(list))
        .route("/static-websites/{id}", get(details))
        .route("/static-websites/{id}/preview/", get(preview_index))
        .route("/static-websites/{id}/preview/{*path}", get(preview_file))
        .route_layer(from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ));
    let writes = Router::new()
        .route("/environments/{id}/static-websites", post(create))
        .route("/static-websites/{id}", axum::routing::delete(remove))
        .route("/static-websites/{id}/domain", post(domain))
        .route(
            "/static-websites/{id}/publish",
            post(publish).layer(DefaultBodyLimit::max(2 * MAX_BYTES + 1024 * 1024)),
        )
        .route("/static-websites/{id}/rollback", post(rollback))
        .route_layer(from_fn_with_state(state, crate::auth::sessions::protect));
    reads.merge(writes)
}

#[derive(Serialize, sqlx::FromRow)]
struct Site {
    id: Uuid,
    project_id: Uuid,
    name: String,
    domain: Option<String>,
    active_release_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

const SITES: &str = "SELECT s.id,e.project_id,s.name,s.domain,s.active_release_id,s.created_at
    FROM static_websites s JOIN environments e ON e.id=s.environment_id
    JOIN projects p ON p.id=e.project_id WHERE p.organization_id=$1";

fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Website or environment not found.")
}

fn invalid(message: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, message)
}

fn conflict(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        ApiError(
            StatusCode::CONFLICT,
            "This name or domain is already in use.",
        )
    } else {
        error.into()
    }
}

async fn list(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
) -> Result<Json<Vec<Site>>, ApiError> {
    Ok(Json(
        sqlx::query_as(&format!("{SITES} AND e.id=$2 ORDER BY s.created_at DESC"))
            .bind(id.organization_id)
            .bind(environment)
            .fetch_all(&state.pool)
            .await?,
    ))
}

#[derive(Deserialize)]
struct NewSite {
    name: String,
}

async fn create(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
    Json(input): Json<NewSite>,
) -> Result<(StatusCode, Json<Site>), ApiError> {
    id.manager()?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(invalid("Enter a website name between 1 and 80 characters."));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM auth_organizations WHERE id=$1 FOR UPDATE")
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM static_websites s
        JOIN environments e ON e.id=s.environment_id JOIN projects p ON p.id=e.project_id
        WHERE p.organization_id=$1",
    )
    .bind(id.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= 10 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "An organization can have up to 10 static websites.",
        ));
    }
    let site = sqlx::query_as(
        "WITH created AS (
        INSERT INTO static_websites(id,environment_id,name)
        SELECT $1,e.id,$2 FROM environments e JOIN projects p ON p.id=e.project_id
        WHERE e.id=$3 AND p.organization_id=$4 RETURNING *
    ) SELECT s.id,e.project_id,s.name,s.domain,s.active_release_id,s.created_at
      FROM created s JOIN environments e ON e.id=s.environment_id",
    )
    .bind(Uuid::new_v4())
    .bind(name)
    .bind(environment)
    .bind(id.organization_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(conflict)?
    .ok_or_else(missing)?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(site)))
}

#[derive(Serialize, sqlx::FromRow)]
struct Release {
    id: Uuid,
    bytes: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn details(
    State(state): State<AppState>,
    id: Identity,
    Path(site): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let site: Site = sqlx::query_as(&format!("{SITES} AND s.id=$2"))
        .bind(id.organization_id)
        .bind(site)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(missing)?;
    let releases: Vec<Release> = sqlx::query_as("SELECT id,bytes,created_at FROM static_releases WHERE site_id=$1 ORDER BY created_at DESC,id DESC")
        .bind(site.id).fetch_all(&state.pool).await?;
    Ok(Json(
        serde_json::json!({ "site": site, "releases": releases }),
    ))
}

// Serialize pointer changes and retention for the same site.
async fn lock_site(
    tx: &mut Transaction<'_, Postgres>,
    id: &Identity,
    site: Uuid,
) -> Result<(), ApiError> {
    let row: Option<Site> = sqlx::query_as(&format!("{SITES} AND s.id=$2 FOR UPDATE OF s"))
        .bind(id.organization_id)
        .bind(site)
        .fetch_optional(&mut **tx)
        .await?;
    row.ok_or_else(missing).map(|_| ())
}

#[derive(Deserialize)]
struct RemoveSite {
    name: String,
}

async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path(site): Path<Uuid>,
    Json(input): Json<RemoveSite>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    let mut tx = state.pool.begin().await?;
    lock_site(&mut tx, &id, site).await?;
    let name: String = sqlx::query_scalar("SELECT name FROM static_websites WHERE id=$1")
        .bind(site)
        .fetch_one(&mut *tx)
        .await?;
    if name != input.name {
        return Err(invalid("Enter the website name to confirm deletion."));
    }
    sqlx::query("DELETE FROM static_websites WHERE id=$1")
        .bind(site)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn hostname(value: &str) -> Option<String> {
    let name = value.trim().trim_end_matches('.').to_ascii_lowercase();
    (name.len() <= 253
        && name.contains('.')
        && name
            .rsplit('.')
            .next()?
            .bytes()
            .any(|b| b.is_ascii_alphabetic())
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        }))
    .then_some(name)
}

#[derive(Deserialize)]
struct Domain {
    domain: Option<String>,
}

async fn domain(
    State(state): State<AppState>,
    id: Identity,
    Path(site): Path<Uuid>,
    Json(input): Json<Domain>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    id.admin()?;
    let domain = input
        .domain
        .as_deref()
        .map(|value| {
            hostname(value)
                .ok_or_else(|| invalid("Enter a DNS hostname without a scheme, wildcard or port."))
        })
        .transpose()?;
    if let Some(name) = &domain {
        let origin = url::Url::parse(&state.auth.origin)
            .map_err(|_| invalid("Dashboard origin is invalid."))?;
        if let Some(host) = origin.host_str() {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            if *name == host
                || host.ends_with(&format!(".{name}"))
                || name.ends_with(&format!(".{host}"))
            {
                return Err(invalid(
                    "Use a hostname separate from the dashboard domain.",
                ));
            }
        }
    }
    let mut tx = state.pool.begin().await?;
    lock_site(&mut tx, &id, site).await?;
    sqlx::query("UPDATE static_websites SET domain=$2 WHERE id=$1")
        .bind(site)
        .bind(domain)
        .execute(&mut *tx)
        .await
        .map_err(conflict)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Upload {
    files: Vec<UploadFile>,
}

#[derive(Deserialize)]
struct UploadFile {
    path: String,
    hex: String,
}

fn valid_path(path: &str) -> bool {
    path.len() <= 512
        && path.split('/').next() != Some("_ferrix")
        && !path.contains(['\\', '?', '#'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.'))
}

struct Build {
    files: Vec<(String, Vec<u8>)>,
    bytes: i32,
}

fn decode_build(input: Upload) -> Result<Build, ApiError> {
    if input.files.len() > 512 {
        return Err(invalid("A build can contain up to 512 files."));
    }
    let mut paths = std::collections::HashSet::new();
    let mut bytes = 0;
    let mut files = Vec::new();
    for file in input.files {
        if !valid_path(&file.path) || !paths.insert(file.path.clone()) {
            return Err(invalid("Build contains an invalid or duplicate path."));
        }
        bytes += file.hex.len() / 2;
        if bytes > MAX_BYTES {
            return Err(invalid("A build can contain up to 8 MiB of files."));
        }
        let content =
            hex::decode(&file.hex).map_err(|_| invalid("Build contains invalid file data."))?;
        files.push((file.path, content));
    }
    if !files
        .iter()
        .any(|(path, content)| path == "index.html" && !content.is_empty())
    {
        return Err(invalid("Select a build folder containing index.html."));
    }
    Ok(Build {
        files,
        bytes: bytes as i32,
    })
}

async fn activate(
    tx: &mut Transaction<'_, Postgres>,
    site: Uuid,
    release: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE static_websites SET active_release_id=$2 WHERE id=$1")
        .bind(site)
        .bind(release)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(Serialize)]
struct ReleaseId {
    id: Uuid,
}

async fn publish(
    State(state): State<AppState>,
    id: Identity,
    Path(site): Path<Uuid>,
    Json(input): Json<Upload>,
) -> Result<(StatusCode, Json<ReleaseId>), ApiError> {
    id.manager()?;
    let Build { files, bytes } = decode_build(input)?;
    let mut tx = state.pool.begin().await?;
    lock_site(&mut tx, &id, site).await?;
    let release = Uuid::new_v4();
    sqlx::query("INSERT INTO static_releases(site_id,id,bytes) VALUES($1,$2,$3)")
        .bind(site)
        .bind(release)
        .bind(bytes)
        .execute(&mut *tx)
        .await?;
    let (paths, contents): (Vec<_>, Vec<_>) = files.into_iter().unzip();
    sqlx::query(
        "INSERT INTO static_files(site_id,release_id,path,content)
        SELECT $1,$2,path,content FROM unnest($3::text[],$4::bytea[]) AS file(path,content)",
    )
    .bind(site)
    .bind(release)
    .bind(paths)
    .bind(contents)
    .execute(&mut *tx)
    .await?;
    activate(&mut tx, site, release).await?;
    sqlx::query(
        "DELETE FROM static_releases WHERE site_id=$1 AND id IN (
        SELECT id FROM static_releases WHERE site_id=$1 AND id<>$2
        ORDER BY created_at DESC,id DESC OFFSET 4)",
    )
    .bind(site)
    .bind(release)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(ReleaseId { id: release })))
}

#[derive(Deserialize)]
struct Rollback {
    release_id: Uuid,
}

async fn rollback(
    State(state): State<AppState>,
    id: Identity,
    Path(site): Path<Uuid>,
    Json(input): Json<Rollback>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    let mut tx = state.pool.begin().await?;
    lock_site(&mut tx, &id, site).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM static_releases WHERE site_id=$1 AND id=$2)",
    )
    .bind(site)
    .bind(input.release_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Release not found for this website.",
        ));
    }
    activate(&mut tx, site, input.release_id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn public_router(pool: PgPool) -> Router {
    Router::new()
        .route("/_ferrix/tls-ask", get(tls_ask))
        .fallback(get(file))
        .with_state(pool)
}

async fn tls_ask(
    State(pool): State<PgPool>,
    Query(input): Query<std::collections::HashMap<String, String>>,
) -> Result<StatusCode, ApiError> {
    let name = input
        .get("domain")
        .and_then(|value| hostname(value))
        .ok_or_else(missing)?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM static_websites WHERE domain=$1 AND active_release_id IS NOT NULL)")
        .bind(name).fetch_one(&pool).await?;
    if !allowed {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}

fn request_path(path: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(path.len().min(1536));
    if path.len() > 1536 {
        return None;
    }
    let mut bytes = path.strip_prefix('/')?.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let mut value = [0];
            hex::decode_to_slice([bytes.next()?, bytes.next()?], &mut value).ok()?;
            if matches!(value[0], b'/' | b'\\') {
                return None;
            }
            decoded.push(value[0]);
        } else {
            decoded.push(byte);
        }
    }
    let mut path = String::from_utf8(decoded).ok()?;
    if path.is_empty() || path.ends_with('/') {
        path.push_str("index.html");
    }
    valid_path(&path).then_some(path)
}

async fn preview_index(
    State(state): State<AppState>,
    id: Identity,
    Path(site): Path<Uuid>,
) -> Result<Response, ApiError> {
    preview(&state, &id, site, "index.html").await
}

async fn preview_file(
    State(state): State<AppState>,
    id: Identity,
    Path((site, path)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    if !valid_path(&path) {
        return Err(missing());
    }
    preview(&state, &id, site, &path).await
}

async fn preview(
    state: &AppState,
    id: &Identity,
    site: Uuid,
    path: &str,
) -> Result<Response, ApiError> {
    let content: Option<Vec<u8>> = sqlx::query_scalar("SELECT f.content FROM static_websites s JOIN environments e ON e.id=s.environment_id JOIN projects p ON p.id=e.project_id JOIN static_files f ON f.site_id=s.id AND f.release_id=s.active_release_id WHERE s.id=$1 AND p.organization_id=$2 AND f.path=$3")
        .bind(site).bind(id.organization_id).bind(path).fetch_optional(&state.pool).await?;
    let mut response = static_response(path, content.ok_or_else(missing)?);
    // Uploaded HTML must not execute with dashboard privileges or submit credentialed forms.
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    Ok(response)
}

async fn file(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
        .and_then(|authority| hostname(authority.host()))
        .ok_or_else(missing)?;
    let path = request_path(uri.path()).ok_or_else(missing)?;
    let content: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT f.content FROM static_websites s JOIN static_files f
        ON f.site_id=s.id AND f.release_id=s.active_release_id WHERE s.domain=$1 AND f.path=$2",
    )
    .bind(host)
    .bind(&path)
    .fetch_optional(&pool)
    .await?;
    let content = content.ok_or(ApiError(StatusCode::NOT_FOUND, "Page not found."))?;
    Ok(static_response(&path, content))
}

fn static_response(path: &str, content: Vec<u8>) -> Response {
    let mime = match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" | "webmanifest" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        content,
    )
        .into_response()
}

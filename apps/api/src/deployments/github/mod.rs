//! GitHub credentials stay server-side; jobs snapshot their source and build immutable commits.
pub(super) mod build;
use super::*;
use axum::{
    body::Bytes,
    http::{HeaderMap, header},
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Source {
    pub repository: String,
    pub branch: String,
    pub dockerfile: String,
    pub context: String,
    pub token: Option<String>,
}
#[derive(sqlx::FromRow)]
struct Saved {
    enabled: bool,
    repository: String,
    branch: String,
    config_cipher: Vec<u8>,
    revision: Uuid,
    auto_deploy: bool,
    webhook_cipher: Option<Vec<u8>>,
    configured_by: Option<Uuid>,
    pending_delivery: Option<Uuid>,
    pending_sha: Option<String>,
    pending_sender: Option<String>,
    last_received_at: Option<DateTime<Utc>>,
    last_message: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    mode: String,
    #[serde(default)]
    upload_id: Option<Uuid>,
    #[serde(default)]
    repository: String,
    #[serde(default = "default_branch")]
    branch: String,
    #[serde(default = "default_dockerfile")]
    dockerfile: String,
    #[serde(default = "default_context")]
    context: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    clear_token: bool,
    #[serde(default)]
    auto_deploy: bool,
}
fn default_branch() -> String {
    "main".into()
}
fn default_dockerfile() -> String {
    "Dockerfile".into()
}
fn default_context() -> String {
    ".".into()
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/source", get(get_source).post(save))
        .route("/services/{id}/source/test", post(test))
        .route("/services/{id}/source/webhook-secret", post(webhook_secret))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
pub fn webhook_routes(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/webhooks/github/{id}", post(webhook))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
}
async fn access(state: &AppState, id: &Identity, service: Uuid) -> Result<(), ApiError> {
    crate::auth::permissions::developer(state, id, service).await?;
    if app_access(state, id, service).await?.kind != "application" {
        return Err(missing());
    }
    Ok(())
}
fn repository(input: &str) -> Result<String, ApiError> {
    let input = input.trim();
    let name = if input.starts_with("https://") {
        let url = url::Url::parse(input)
            .map_err(|_| invalid("Use a GitHub repository URL or owner/repository."))?;
        if url.host_str() != Some("github.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "Use an HTTPS github.com repository URL without credentials or query parameters.",
            ));
        }
        url.path().trim_matches('/').to_owned()
    } else {
        input.to_owned()
    };
    let name = name.strip_suffix(".git").unwrap_or(&name);
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() != 2
        || parts[0].is_empty()
        || parts[0].len() > 39
        || !parts[0]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || parts[1].is_empty()
        || parts[1].len() > 100
        || matches!(parts[1], "." | "..")
        || !parts[1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(invalid(
            "Use a GitHub owner/repository, for example octocat/hello-world.",
        ));
    }
    Ok(name.to_ascii_lowercase())
}
pub(super) fn path(input: &str, root: bool) -> Result<String, ApiError> {
    let value = input.trim().trim_end_matches('/');
    if root && matches!(value, "" | ".") {
        return Ok(".".into());
    }
    if value.is_empty()
        || value.len() > 255
        || value.starts_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|p| p.is_empty() || matches!(p, ".." | "."))
    {
        return Err(invalid(
            "Use a relative path inside the repository without parent-directory segments.",
        ));
    }
    Ok(value.to_owned())
}
fn branch(input: &str) -> Result<String, ApiError> {
    let value = input.trim();
    if value.is_empty()
        || value.len() > 200
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value
            .split('/')
            .any(|p| p.is_empty() || p.starts_with('.') || p.ends_with(".lock"))
        || value
            .bytes()
            .any(|b| b <= 32 || b == 127 || b"~^:?*[\\".contains(&b))
    {
        return Err(invalid("Use a valid Git branch name."));
    }
    Ok(value.into())
}
fn seal<T: Serialize>(
    state: &AppState,
    app: Uuid,
    purpose: &str,
    value: &T,
) -> Result<Vec<u8>, ApiError> {
    use rand::RngCore;
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let plain = serde_json::to_vec(value).map_err(|_| internal())?;
    let aad = format!("ferrix:github:{purpose}:{app}");
    let cipher = XChaCha20Poly1305::new((&state.auth.key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    Ok([nonce.to_vec(), cipher].concat())
}
fn open<T: serde::de::DeserializeOwned>(
    state: &AppState,
    app: Uuid,
    purpose: &str,
    cipher: &[u8],
) -> Result<T, ApiError> {
    if cipher.len() < 40 {
        return Err(internal());
    }
    let aad = format!("ferrix:github:{purpose}:{app}");
    let plain = XChaCha20Poly1305::new((&state.auth.key).into())
        .decrypt(
            XNonce::from_slice(&cipher[..24]),
            Payload {
                msg: &cipher[24..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    serde_json::from_slice(&plain).map_err(|_| internal())
}
async fn get_source(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    access(&state, &id, app).await?;
    let row: Option<Saved> =
        sqlx::query_as("SELECT * FROM application_git_sources WHERE application_id=$1")
            .bind(app)
            .fetch_optional(&state.pool)
            .await?;
    let url = format!("{}/api/v1/webhooks/github/{app}", state.auth.origin);
    if let Some(upload) = uploads::current(&state, app).await? {
        return Ok(Json(upload));
    }
    if let Some(row) = row {
        let source: Source = open(&state, app, "source", &row.config_cipher)?;
        Ok(Json(
            json!({"mode":if row.enabled{"github"}else{"image"},"repository":source.repository,"branch":source.branch,"dockerfile":source.dockerfile,"context":source.context,"token_saved":source.token.is_some(),"auto_deploy":row.auto_deploy,"webhook_configured":row.webhook_cipher.is_some(),"webhook_url":url,"pending":row.pending_delivery.is_some(),"pending_commit":row.pending_sha,"last_received_at":row.last_received_at,"last_message":row.last_message}),
        ))
    } else {
        Ok(Json(
            json!({"mode":"image","repository":"","branch":"main","dockerfile":"Dockerfile","context":".","token_saved":false,"auto_deploy":false,"webhook_configured":false,"webhook_url":url,"pending":false,"last_received_at":null,"last_message":null}),
        ))
    }
}
async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    Json(edit): Json<Edit>,
) -> Result<StatusCode, ApiError> {
    access(&state, &id, app).await?;
    if !matches!(edit.mode.as_str(), "image" | "github" | "upload") {
        return Err(invalid("Choose Docker image or GitHub source."));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .execute(&mut *tx)
        .await?;
    if edit.mode != "upload" {
        sqlx::query("UPDATE applications SET source_upload_id=NULL WHERE id=$1")
            .bind(app)
            .execute(&mut *tx)
            .await?;
    }
    if edit.mode == "upload" {
        let revision = edit
            .upload_id
            .ok_or_else(|| invalid("Preview an archive first."))?;
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_source_uploads WHERE id=$1 AND application_id=$2)").bind(revision).bind(app).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(invalid("This uploaded revision is unavailable."));
        }
        sqlx::query("UPDATE applications SET source_upload_id=$2,updated_at=now() WHERE id=$1")
            .bind(app)
            .bind(revision)
            .execute(&mut *tx)
            .await?;
    }
    if edit.mode != "github" {
        sqlx::query("UPDATE application_git_sources SET enabled=false,auto_deploy=false,pending_delivery=NULL,pending_sha=NULL,pending_sender=NULL,updated_at=now() WHERE application_id=$1").bind(app).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let repo = repository(&edit.repository)?;
    let branch = branch(&edit.branch)?;
    let dockerfile = path(&edit.dockerfile, false)?;
    let context = path(&edit.context, true)?;
    let previous: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT config_cipher FROM application_git_sources WHERE application_id=$1",
    )
    .bind(app)
    .fetch_optional(&mut *tx)
    .await?;
    let previous = previous
        .as_deref()
        .map(|c| open::<Source>(&state, app, "source", c))
        .transpose()?;
    if edit.clear_token && edit.token.as_ref().is_some_and(|t| !t.is_empty()) {
        return Err(invalid("Choose a new token or clear the saved token."));
    }
    if !crate::auth::permissions::manager(&id)
        && !edit.clear_token
        && edit.token.as_ref().is_none_or(|token| token.is_empty())
        && previous
            .as_ref()
            .is_some_and(|source| source.token.is_some() && source.repository != repo)
    {
        return Err(invalid(
            "Provide your own token or clear the saved token when changing repositories.",
        ));
    }
    let token = if edit.clear_token {
        None
    } else if let Some(token) = edit.token.filter(|t| !t.is_empty()) {
        if token.len() > 512 || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(invalid("Use a valid GitHub token without whitespace."));
        }
        Some(token)
    } else {
        previous.and_then(|s| s.token)
    };
    let source = Source {
        repository: repo.clone(),
        branch: branch.clone(),
        dockerfile,
        context,
        token,
    };
    let cipher = seal(&state, app, "source", &source)?;
    sqlx::query("INSERT INTO application_git_sources(application_id,repository,branch,config_cipher,revision,auto_deploy,configured_by) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(application_id) DO UPDATE SET enabled=true,repository=EXCLUDED.repository,branch=EXCLUDED.branch,config_cipher=EXCLUDED.config_cipher,revision=EXCLUDED.revision,auto_deploy=EXCLUDED.auto_deploy,configured_by=EXCLUDED.configured_by,pending_delivery=NULL,pending_sha=NULL,pending_sender=NULL,last_message=NULL,updated_at=now()")
 .bind(app).bind(repo).bind(branch).bind(cipher).bind(Uuid::new_v4()).bind(edit.auto_deploy).bind(id.user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn test(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    access(&state, &id, app).await?;
    crate::auth::accounts::throttle(&state.pool, &format!("github-test:{}", id.user_id), 10)
        .await?;
    let cipher: Vec<u8> = sqlx::query_scalar(
        "SELECT config_cipher FROM application_git_sources WHERE application_id=$1 AND enabled",
    )
    .bind(app)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| invalid("Save a GitHub source first."))?;
    let source: Source = open(&state, app, "source", &cipher)?;
    let commit=build::resolve(&source).await.map_err(|_|invalid("Could not access this GitHub repository/branch. Check the token's Contents read permission."))?;
    Ok(Json(
        json!({"message":"Repository and branch are accessible.","commit":commit}),
    ))
}
async fn webhook_secret(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    access(&state, &id, app).await?;
    id.fresh()?;
    let secret = crate::auth::token();
    let cipher = seal(&state, app, "webhook", &secret)?;
    let updated = sqlx::query(
        "UPDATE application_git_sources SET webhook_cipher=$2 WHERE application_id=$1 AND enabled",
    )
    .bind(app)
    .bind(cipher)
    .execute(&state.pool)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(invalid(
            "Save a GitHub source before generating a webhook secret.",
        ));
    }
    Ok(Json(
        json!({"secret":secret,"url":format!("{}/api/v1/webhooks/github/{app}",state.auth.origin)}),
    ))
}
#[derive(Deserialize)]
struct Push {
    #[serde(rename = "ref")]
    reference: String,
    after: String,
    #[serde(default)]
    deleted: bool,
    repository: PushRepository,
    sender: Option<Sender>,
}
#[derive(Deserialize)]
struct PushRepository {
    full_name: String,
}
#[derive(Deserialize)]
struct Sender {
    login: String,
}
fn header_one<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}
fn verify(secret: &str, signature: &str, body: &[u8]) -> bool {
    let Some(value) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(value) = hex::decode(value) else {
        return false;
    };
    let Ok(mut mac) = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&value).is_ok()
}
fn commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.bytes().any(|b| b != b'0')
}
async fn webhook(
    State(state): State<AppState>,
    Path(app): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if headers.contains_key(header::AUTHORIZATION) {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Use the GitHub webhook signature.",
        ));
    }
    let row: Option<Saved> =
        sqlx::query_as("SELECT * FROM application_git_sources WHERE application_id=$1 AND enabled")
            .bind(app)
            .fetch_optional(&state.pool)
            .await?;
    let row = row.ok_or_else(missing)?;
    let secret: String = open(
        &state,
        app,
        "webhook",
        row.webhook_cipher.as_deref().ok_or(ApiError(
            StatusCode::UNAUTHORIZED,
            "Webhook signature required.",
        ))?,
    )?;
    if !header_one(&headers, "x-hub-signature-256").is_some_and(|sig| verify(&secret, sig, &body)) {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Invalid GitHub webhook signature.",
        ));
    }
    let event =
        header_one(&headers, "x-github-event").ok_or_else(|| invalid("Missing GitHub event."))?;
    let delivery = header_one(&headers, "x-github-delivery")
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| invalid("Use a valid GitHub delivery ID."))?;
    if event == "ping" {
        return Ok((StatusCode::OK, Json(json!({"status":"ready"}))));
    }
    if event != "push" || !row.auto_deploy {
        return Ok((StatusCode::OK, Json(json!({"status":"ignored"}))));
    }
    let push: Push =
        serde_json::from_slice(&body).map_err(|_| invalid("Invalid GitHub push payload."))?;
    if push.deleted
        || push.reference != format!("refs/heads/{}", row.branch)
        || !push
            .repository
            .full_name
            .eq_ignore_ascii_case(&row.repository)
    {
        return Ok((StatusCode::OK, Json(json!({"status":"ignored"}))));
    }
    if !commit(&push.after) {
        return Err(invalid("Invalid push commit SHA."));
    }
    let sender = push.sender.map(|s| {
        s.login
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect::<String>()
    });
    let mut tx = state.pool.begin().await?;
    // Serialize with source edits without taking the application lock on the public endpoint.
    type ReceiptGate = (Uuid, bool, Option<Vec<u8>>);
    let current:Option<ReceiptGate>=sqlx::query_as("SELECT revision,auto_deploy,webhook_cipher FROM application_git_sources WHERE application_id=$1 AND enabled FOR UPDATE").bind(app).fetch_optional(&mut *tx).await?;
    if current != Some((row.revision, true, row.webhook_cipher.clone())) {
        return Ok((StatusCode::OK, Json(json!({"status":"ignored"}))));
    }
    let inserted=sqlx::query("INSERT INTO github_webhook_deliveries(application_id,delivery_id,payload_hash) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(app).bind(delivery).bind(hex::encode(Sha256::digest(&body))).execute(&mut *tx).await?.rows_affected();
    if inserted == 0 {
        return Ok((StatusCode::OK, Json(json!({"status":"duplicate"}))));
    }
    sqlx::query("UPDATE application_git_sources SET pending_delivery=$2,pending_sha=$3,pending_sender=$4,last_received_at=now(),last_message='Push received. Latest branch head will deploy when the service is idle.' WHERE application_id=$1").bind(app).bind(delivery).bind(push.after).bind(sender).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"status":"accepted"}))))
}
pub(super) async fn snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app: Uuid,
    deployment: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE deployments d SET source_cipher=s.config_cipher,source_revision=s.revision,source_repository=s.repository,source_branch=s.branch FROM application_git_sources s WHERE d.id=$1 AND s.application_id=$2 AND s.enabled").bind(deployment).bind(app).execute(&mut **tx).await?;
    sqlx::query("UPDATE deployments d SET source_upload_id=a.source_upload_id,source_revision=a.source_upload_id,source_archive_sha256=(SELECT sha256 FROM application_source_uploads WHERE id=a.source_upload_id) FROM applications a WHERE d.id=$1 AND a.id=$2 AND a.source_upload_id IS NOT NULL").bind(deployment).bind(app).execute(&mut **tx).await?;
    Ok(())
}
pub(super) async fn dispatch(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<(), String> {
    use sqlx::Acquire;
    let mut tx = conn
        .begin()
        .await
        .map_err(|_| "Could not start GitHub deployment queue.")?;
    let mut reserved = Some(
        quotas::reserve(state, &mut tx)
            .await
            .map_err(|_| "Could not reserve quota capacity.")?,
    );
    let apps:Vec<Uuid>=sqlx::query_scalar("SELECT a.id FROM applications a JOIN application_git_sources s ON s.application_id=a.id WHERE NOT EXISTS(SELECT 1 FROM project_maintenance m JOIN environments e ON e.project_id=m.project_id WHERE e.id=a.environment_id) AND NOT a.cutover_pending AND NOT a.restore_blocked AND s.enabled AND s.auto_deploy AND s.pending_delivery IS NOT NULL AND NOT EXISTS(SELECT 1 FROM database_jobs j WHERE j.status IN ('queued','running') AND (j.application_id=a.id OR j.source_application_id=a.id)) AND NOT EXISTS(SELECT 1 FROM deployments d WHERE d.application_id=a.id AND d.status IN ('queued','running')) ORDER BY s.last_received_at LIMIT 1 FOR UPDATE OF a SKIP LOCKED").fetch_all(&mut *tx).await.map_err(|_|"Could not claim pending GitHub push.")?;
    for app in apps {
        let row: Saved = sqlx::query_as(
            "SELECT * FROM application_git_sources WHERE application_id=$1 FOR UPDATE",
        )
        .bind(app)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Could not read GitHub source.")?;
        if !row.enabled || !row.auto_deploy || row.pending_delivery.is_none() {
            continue;
        }
        let name:Option<String>=sqlx::query_scalar("SELECT u.name FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id JOIN auth_members m ON m.organization_id=p.organization_id JOIN auth_users u ON u.id=m.user_id WHERE a.id=$1 AND u.id=$2 AND ferrix_service_permission(u.id,p.organization_id,a.id) IN ('manager','developer') AND NOT auth_user_banned(u.banned,u.ban_expires_at)").bind(app).bind(row.configured_by).fetch_optional(&mut *tx).await.map_err(|_|"Could not check auto-deploy permissions.")?;
        let message = if let Some(ref name) = name {
            let deployment = Uuid::new_v4();
            sqlx::query("INSERT INTO deployments(id,application_id,requested_by,requested_by_name,image,container_port,env_cipher,service_kind,worker_mode,service_cipher,source_cipher,source_revision,source_repository,source_branch,trigger_kind,trigger_sender) SELECT $1,id,$2,$3,image,container_port,env_cipher,service_kind,worker_mode,service_cipher,$4,$5,$6,$7,'webhook',$8 FROM applications WHERE id=$9 AND service_kind='application'").bind(deployment).bind(row.configured_by).bind(name).bind(row.config_cipher).bind(row.revision).bind(row.repository).bind(row.branch).bind(row.pending_sender).bind(app).execute(&mut *tx).await.map_err(|_|"Could not queue GitHub deployment.")?;
            if let Err(error) = quotas::check(
                state,
                &mut tx,
                reserved.take().ok_or("Missing quota reservation.")?,
            )
            .await
            {
                tx.rollback()
                    .await
                    .map_err(|_| "Could not release quota reservation.")?;
                if error.0 != StatusCode::CONFLICT {
                    return Err("Could not check deployment quota.".into());
                }
                sqlx::query("UPDATE application_git_sources SET last_message='Auto-deploy waiting for available project/organization quota.' WHERE application_id=$1").bind(app).execute(&mut *conn).await.map_err(|_|"Could not report quota wait.")?;
                return Ok(());
            }
            "GitHub deployment queued."
        } else {
            "Auto-deploy paused: the configuring user no longer has permission."
        };
        sqlx::query("UPDATE application_git_sources SET pending_delivery=NULL,pending_sha=NULL,pending_sender=NULL,last_message=$2,auto_deploy=CASE WHEN $3 THEN auto_deploy ELSE false END WHERE application_id=$1").bind(app).bind(message).bind(name.is_some()).execute(&mut *tx).await.map_err(|_|"Could not record GitHub queue status.")?;
    }
    tx.commit()
        .await
        .map_err(|_| "Could not persist GitHub deployment.")?;
    let _ = state;
    Ok(())
}
pub(super) async fn build_image(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    job: &worker::Job,
) -> Result<bollard::models::ImageInspect, String> {
    build::run(state, conn, job).await
}

#[cfg(test)]
use super::test_support;
#[cfg(test)]
mod tests;

// Copy build settings with a new encryption context; webhook state never transfers.
pub(super) async fn clone_source(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    source: Uuid,
    target: Uuid,
    user: Uuid,
) -> Result<(), ApiError> {
    let row: Option<(bool, Vec<u8>)> = sqlx::query_as(
        "SELECT enabled,config_cipher FROM application_git_sources WHERE application_id=$1",
    )
    .bind(source)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((enabled, cipher)) = row {
        let config: Source = open(state, source, "source", &cipher)?;
        let cipher = seal(state, target, "source", &config)?;
        sqlx::query("INSERT INTO application_git_sources(application_id,enabled,repository,branch,config_cipher,revision,configured_by) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(target).bind(enabled).bind(config.repository).bind(config.branch).bind(cipher).bind(Uuid::new_v4()).bind(user).execute(conn).await?;
    }
    Ok(())
}

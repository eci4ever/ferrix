mod backups;
mod domains;
pub(crate) mod observability;
mod services;
mod terminal;
mod worker;
pub use terminal::run_terminal_worker;
pub use worker::run_worker;

use crate::{ApiError, AppState, auth::Identity};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

pub fn routes(state: AppState) -> Router<AppState> {
    let reads = Router::new()
        .route("/projects/{id}/environments", get(environments))
        .route("/environments/{id}/applications", get(applications))
        .route("/applications/{id}", get(application))
        .route("/applications/{id}/configuration", get(configuration))
        .route("/applications/{id}/deployments", get(history))
        .route("/applications/{id}/runtime", get(runtime))
        .route("/applications/{id}/logs", get(logs))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ));
    reads
        .merge(services::routes(state.clone()))
        .merge(domains::routes(state.clone()))
        .merge(backups::routes(state.clone()))
        .merge(observability::routes(state.clone()))
        .merge(terminal::routes(state.clone()))
        .merge(
            Router::new()
                .route("/projects/{id}/environments", post(create_environment))
                .route("/environments/{id}/applications", post(create_application))
                .route("/applications/{id}/configuration", post(save_configuration))
                .route("/applications/{id}/deploy", post(deploy))
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
        .layer(DefaultBodyLimit::max(256 * 1024))
}
fn invalid(message: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, message)
}
fn missing() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "Resource not found in this organization.",
    )
}
fn internal() -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Could not process application configuration.",
    )
}
fn name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 {
        return Err(invalid("Name must contain 1–80 characters."));
    }
    Ok(value.to_owned())
}
fn database_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        ApiError(
            StatusCode::CONFLICT,
            "This name already exists, or a deployment is already queued/running.",
        )
    } else {
        error.into()
    }
}
#[derive(Serialize, sqlx::FromRow)]
struct Environment {
    id: Uuid,
    project_id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
}
#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct Application {
    #[sqlx(default)]
    automatic_url: Option<String>,
    database_engine: Option<String>,
    kind: String,
    deployed: bool,
    active_deployment_id: Option<Uuid>,
    active_image: Option<String>,
    pending_changes: bool,
    database_operation_pending: bool,
    id: Uuid,
    environment_id: Uuid,
    project_id: Uuid,
    name: String,
    image: String,
    container_port: i32,
    active_container_id: Option<String>,
    host_port: Option<i32>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
#[derive(Serialize, sqlx::FromRow)]
struct Deployment {
    id: Uuid,
    application_id: Uuid,
    image: String,
    container_port: i32,
    status: String,
    logs: String,
    container_id: Option<String>,
    image_id: Option<String>,
    rollback_of: Option<Uuid>,
    requested_by: Option<Uuid>,
    requested_by_name: Option<String>,
    requested_by_key_id: Option<Uuid>,
    requested_by_key_name: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}
const APP_COLUMNS: &str = "CASE WHEN a.service_kind='database' THEN CASE WHEN a.image LIKE 'mongo:%' THEN 'mongodb' WHEN a.image LIKE 'mariadb:%' THEN 'mariadb' WHEN a.image LIKE 'redis:%' THEN 'redis' ELSE 'postgres' END END AS database_engine,a.service_kind AS kind,(a.active_deployment_id IS NOT NULL) AS deployed,a.active_deployment_id,(SELECT image FROM deployments WHERE id=a.active_deployment_id) AS active_image,EXISTS(SELECT 1 FROM deployments d WHERE d.id=a.active_deployment_id AND (a.image IS DISTINCT FROM d.image OR a.container_port IS DISTINCT FROM d.container_port OR a.env_cipher IS DISTINCT FROM d.env_cipher OR a.service_cipher IS DISTINCT FROM d.service_cipher)) AS pending_changes,EXISTS(SELECT 1 FROM database_jobs j WHERE j.status IN ('queued','running') AND (j.application_id=a.id OR j.source_application_id=a.id)) AS database_operation_pending,a.id,a.environment_id,(SELECT project_id FROM environments WHERE id=a.environment_id) AS project_id,a.name,a.image,a.container_port,a.active_container_id,a.host_port,a.created_at,a.updated_at";
async fn project_access(state: &AppState, id: &Identity, project: Uuid) -> Result<(), ApiError> {
    if !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1 AND organization_id=$2)",
    )
    .bind(project)
    .bind(id.organization_id)
    .fetch_one(&state.pool)
    .await?
    {
        return Err(missing());
    }
    Ok(())
}
async fn environment_access(
    state: &AppState,
    id: &Identity,
    environment: Uuid,
) -> Result<(), ApiError> {
    if !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM environments e JOIN projects p ON p.id=e.project_id WHERE e.id=$1 AND p.organization_id=$2)").bind(environment).bind(id.organization_id).fetch_one(&state.pool).await? { return Err(missing()); }
    Ok(())
}
impl Application {
    fn with_gateway(mut self, state: &AppState) -> Self {
        if self.kind == "application" {
            self.automatic_url = state
                .gateway
                .url(crate::gateway::Resource::Application(self.id));
        }
        self
    }
}
pub(crate) async fn app_access(
    state: &AppState,
    id: &Identity,
    app: Uuid,
) -> Result<Application, ApiError> {
    sqlx::query_as::<_,Application>(&format!("SELECT {APP_COLUMNS} FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2 AND a.service_kind IN ('application','database')"))
        .bind(app).bind(id.organization_id).fetch_optional(&state.pool).await?.map(|app|app.with_gateway(state)).ok_or_else(missing)
}
async fn environments(
    State(state): State<AppState>,
    id: Identity,
    Path(project): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    project_access(&state, &id, project).await?;
    let (project_name,): (String,) = sqlx::query_as("SELECT name FROM projects WHERE id=$1")
        .bind(project)
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query_as::<_, Environment>(
        "SELECT * FROM environments WHERE project_id=$1 ORDER BY created_at,id",
    )
    .bind(project)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"project_name":project_name,"environments":rows}),
    ))
}
#[derive(Deserialize)]
struct Named {
    name: String,
}
async fn create_environment(
    State(state): State<AppState>,
    id: Identity,
    Path(project): Path<Uuid>,
    Json(input): Json<Named>,
) -> Result<(StatusCode, Json<Environment>), ApiError> {
    id.manager()?;
    project_access(&state, &id, project).await?;
    let row =
        sqlx::query_as("INSERT INTO environments(id,project_id,name) VALUES($1,$2,$3) RETURNING *")
            .bind(Uuid::new_v4())
            .bind(project)
            .bind(name(&input.name)?)
            .fetch_one(&state.pool)
            .await
            .map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(row)))
}
async fn applications(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
) -> Result<Json<Vec<Application>>, ApiError> {
    environment_access(&state, &id, environment).await?;
    Ok(Json(sqlx::query_as(&format!("SELECT {APP_COLUMNS} FROM applications a WHERE environment_id=$1 AND service_kind='application' ORDER BY created_at,id")).bind(environment).fetch_all(&state.pool).await?))
}
#[derive(Deserialize)]
struct Configuration {
    name: String,
    image: String,
    container_port: i32,
    #[serde(default)]
    environment: String,
}
impl Configuration {
    fn validate(&mut self) -> Result<Vec<String>, ApiError> {
        self.name = name(&self.name)?;
        self.image = self.image.trim().to_owned();
        if self.image.is_empty()
            || self.image.len() > 255
            || !self
                .image
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-/:@".contains(&b))
            || self.image.starts_with('-')
            || self.image.contains("://")
        {
            return Err(invalid(
                "Enter a Docker image reference, for example nginx:alpine.",
            ));
        }
        if !(1..=65535).contains(&self.container_port) {
            return Err(invalid("Container port must be between 1 and 65535."));
        }
        parse_environment(&self.environment)
    }
}
fn parse_environment(text: &str) -> Result<Vec<String>, ApiError> {
    if text.len() > 16384 {
        return Err(invalid("Environment variables must be at most 16 KiB."));
    }
    let mut keys = HashSet::new();
    let mut values = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| invalid("Use one KEY=value environment variable per line."))?;
        if key.is_empty()
            || !key
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
            || value.contains('\0')
            || !keys.insert(key)
        {
            return Err(invalid(
                "Environment keys must be unique letters, digits and underscores, starting with a letter or underscore.",
            ));
        }
        values.push(line.to_owned());
    }
    if values.len() > 100 {
        return Err(invalid("At most 100 environment variables are supported."));
    }
    Ok(values)
}
fn encrypt(state: &AppState, app: Uuid, values: &[String]) -> Result<Option<Vec<u8>>, ApiError> {
    if values.is_empty() {
        return Ok(None);
    }
    let plain = serde_json::to_vec(values).map_err(|_| internal())?;
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let aad = format!("ferrix:application:{app}");
    let cipher = XChaCha20Poly1305::new((&state.auth.key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    Ok(Some([nonce.to_vec(), cipher].concat()))
}
fn decrypt(state: &AppState, app: Uuid, cipher: Option<&[u8]>) -> Result<Vec<String>, ApiError> {
    let Some(cipher) = cipher else {
        return Ok(Vec::new());
    };
    if cipher.len() < 40 {
        return Err(internal());
    }
    let aad = format!("ferrix:application:{app}");
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
async fn create_application(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
    Json(mut input): Json<Configuration>,
) -> Result<(StatusCode, Json<Application>), ApiError> {
    id.manager()?;
    environment_access(&state, &id, environment).await?;
    let values = input.validate()?;
    let app = Uuid::new_v4();
    let cipher = encrypt(&state, app, &values)?;
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,env_cipher) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(app).bind(environment).bind(input.name).bind(input.image).bind(input.container_port).bind(cipher).execute(&state.pool).await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(app_access(&state, &id, app).await?),
    ))
}
async fn application(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Application>, ApiError> {
    Ok(Json(app_access(&state, &id, app).await?))
}
async fn configuration(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    let row = app_access(&state, &id, app).await?;
    if row.kind != "application" {
        return Err(missing());
    }
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT env_cipher FROM applications WHERE id=$1")
            .bind(app)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(
        json!({"name":row.name,"image":row.image,"container_port":row.container_port,"environment":decrypt(&state,app,cipher.as_deref())?.join("\n")}),
    ))
}
async fn save_configuration(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    Json(mut input): Json<Configuration>,
) -> Result<Json<Application>, ApiError> {
    id.manager()?;
    if app_access(&state, &id, app).await?.kind != "application" {
        return Err(missing());
    }
    let values = input.validate()?;
    let cipher = encrypt(&state, app, &values)?;
    sqlx::query("UPDATE applications SET name=$1,image=$2,container_port=$3,env_cipher=$4,updated_at=now() WHERE id=$5")
        .bind(input.name).bind(input.image).bind(input.container_port).bind(cipher).bind(app).execute(&state.pool).await.map_err(database_error)?;
    Ok(Json(app_access(&state, &id, app).await?))
}
async fn deploy(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    key: Option<axum::Extension<crate::auth::api_keys::KeyAccess>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    id.manager()?;
    app_access(&state, &id, app).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(missing)?;
    backups::ensure_idle(&mut tx, app).await?;
    let deployment = Uuid::new_v4();
    sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port,env_cipher,service_kind,service_cipher,requested_by_name,requested_by_key_id,requested_by_key_name) SELECT $1,id,$2,image,container_port,env_cipher,service_kind,service_cipher,$4,$5,$6 FROM applications WHERE id=$3")
        .bind(deployment).bind(id.user_id).bind(app).bind(&id.name).bind(key.as_ref().map(|key|key.id)).bind(key.as_ref().map(|key|key.name.as_str())).execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id":deployment,"status":"queued"})),
    ))
}
#[derive(Deserialize)]
struct RollbackRequest {
    deployment_id: Uuid,
}
async fn rollback(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    key: Option<axum::Extension<crate::auth::api_keys::KeyAccess>>,
    Json(input): Json<RollbackRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    id.manager()?;
    if app_access(&state, &id, app).await?.kind != "application" {
        return Err(missing());
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM applications WHERE id=$1 AND service_kind='application' FOR UPDATE",
    )
    .bind(app)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(missing)?;
    let deployment = Uuid::new_v4();
    let inserted = sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port,env_cipher,service_kind,service_cipher,rollback_of,pinned_image_id,requested_by_name,requested_by_key_id,requested_by_key_name) SELECT $1,application_id,$2,image,container_port,env_cipher,service_kind,service_cipher,id,image_id,$5,$6,$7 FROM deployments WHERE id=$3 AND application_id=$4 AND status='succeeded' AND service_kind='application' AND image_id IS NOT NULL")
        .bind(deployment).bind(id.user_id).bind(input.deployment_id).bind(app).bind(&id.name).bind(key.as_ref().map(|key|key.id)).bind(key.as_ref().map(|key|key.name.as_str()))
        .execute(&mut *tx).await.map_err(database_error)?;
    if inserted.rows_affected() != 1 {
        return Err(invalid(
            "Choose a successful deployment from this application with a recorded image ID.",
        ));
    }
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id":deployment,"status":"queued"})),
    ))
}
async fn history(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Vec<Deployment>>, ApiError> {
    app_access(&state, &id, app).await?;
    Ok(Json(sqlx::query_as("SELECT id,application_id,image,container_port,status,logs,container_id,image_id,rollback_of,requested_by,requested_by_name,requested_by_key_id,requested_by_key_name,created_at,started_at,finished_at FROM deployments WHERE application_id=$1 ORDER BY created_at DESC,id DESC LIMIT 30").bind(app).fetch_all(&state.pool).await?))
}
async fn runtime(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let row = app_access(&state, &id, app).await?;
    worker::runtime(&state, &row).await.map(Json)
}
async fn logs(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    Query(input): Query<observability::LogQuery>,
) -> Result<Json<Value>, ApiError> {
    let row = app_access(&state, &id, app).await?;
    observability::logs(&state, &row, input).await.map(Json)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_environment_without_shell_expansion() {
        assert_eq!(
            parse_environment("# comment\nURL=https://example.com?a=b\nEMPTY=\nVALUE=$(whoami)")
                .unwrap(),
            vec!["URL=https://example.com?a=b", "EMPTY=", "VALUE=$(whoami)"]
        );
        assert!(parse_environment("A=1\nA=2").is_err());
        assert!(parse_environment("1BAD=x").is_err());
        assert!(parse_environment("BAD KEY=x").is_err());
    }
}

#[cfg(test)]
mod encryption_tests {
    use super::*;
    #[tokio::test]
    async fn environment_cipher_is_bound_to_the_application() {
        let mut state = AppState {
            gateway: Default::default(),
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
                .unwrap(),
            docker: bollard::Docker::connect_with_local_defaults().unwrap(),
            auth: std::sync::Arc::new(crate::auth::AuthConfig {
                origin: "http://127.0.0.1:5173".into(),
                secure: false,
                cookie_name: "test".into(),
                key: [42; 32],
            }),
        };
        let app = Uuid::new_v4();
        let values = vec!["SECRET=example-secret".to_owned()];
        let cipher = encrypt(&state, app, &values).unwrap().unwrap();
        assert_eq!(decrypt(&state, app, Some(&cipher)).unwrap(), values);
        assert!(decrypt(&state, Uuid::new_v4(), Some(&cipher)).is_err());
        let mut tampered = cipher.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decrypt(&state, app, Some(&tampered)).is_err());
        std::sync::Arc::make_mut(&mut state.auth).key = [43; 32];
        assert!(decrypt(&state, app, Some(&cipher)).is_err());
    }
}

pub(crate) async fn inspect_gateway_container(
    state: &AppState,
    app: Uuid,
    container: &str,
) -> Result<bollard::models::ContainerInspectResponse, ApiError> {
    worker::inspect_owned(state, app, container).await
}

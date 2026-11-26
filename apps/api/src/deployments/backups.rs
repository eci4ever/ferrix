mod applications;
mod logical_policy;
mod policy;
mod remote;
use super::*;
use bollard::{
    models::{
        ContainerCreateBody, HostConfig, HostConfigLogConfig, Mount, MountTypeEnum,
        VolumeCreateRequest,
    },
    query_parameters::{
        CreateContainerOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    },
};
use futures_util::StreamExt;
use services::{DatabaseConfig, DatabaseEngine};
use sqlx::PgConnection;
use std::{collections::HashMap, time::Duration};
use tokio::sync::watch;

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(policy::routes(state.clone()))
        .merge(logical_policy::routes(state.clone()))
        .route("/services/{id}/backups", get(list))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ))
        .merge(
            Router::new()
                .route("/services/{id}/backups", post(create))
                .route("/services/{id}/backups/{backup_id}/restore", post(restore))
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
}
#[derive(Serialize, sqlx::FromRow)]
struct Record {
    id: Uuid,
    managed_database_id: Option<Uuid>,
    managed_database_name: Option<String>,
    application_id: Uuid,
    service_name: String,
    project_id: Uuid,
    source_backup_id: Option<Uuid>,
    operation: String,
    engine: String,
    status: String,
    message: String,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    storage: String,
    scheduled_for: Option<DateTime<Utc>>,
    pruning: bool,
    pruned_at: Option<DateTime<Utc>>,
    retention_error: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}
#[derive(sqlx::FromRow)]
struct Job {
    id: Uuid,
    application_id: Uuid,
    #[sqlx(default)]
    managed_database_id: Option<Uuid>,
    source_application_id: Option<Uuid>,
    source_backup_id: Option<Uuid>,
    operation: String,
    engine: String,
    image: String,
    image_id: String,
    application_port: i32,
    config_cipher: Vec<u8>,
    env_cipher: Option<Vec<u8>>,
    #[sqlx(default)]
    s3_cipher: Option<Vec<u8>>,
    #[sqlx(default)]
    s3_key: Option<String>,
}
async fn access(state: &AppState, id: &Identity, service: Uuid) -> Result<Application, ApiError> {
    let app = app_access(state, id, service).await?;
    if app.kind == "application"
        && super::application_runtime::load(state, service)
            .await?
            .storage
            .is_some()
    {
        return Ok(app);
    }
    if app.kind != "database"
        || !matches!(app.database_engine.as_deref(), Some("postgres" | "mariadb"))
    {
        return Err(invalid(
            "Backups require Application persistent storage or a PostgreSQL/MariaDB service.",
        ));
    }
    Ok(app)
}
pub(super) async fn ensure_idle(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    service: Uuid,
) -> Result<(), ApiError> {
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM database_jobs WHERE status IN ('queued','running') AND (application_id=$1 OR source_application_id=$1))")
        .bind(service).fetch_one(&mut **tx).await?;
    if pending {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Wait for backup or restore to finish before changing this service.",
        ));
    }
    Ok(())
}
pub(super) async fn ensure_restored(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    service: Uuid,
) -> Result<(), ApiError> {
    let blocked: bool = sqlx::query_scalar(
        "SELECT restore_blocked OR cutover_pending FROM applications WHERE id=$1",
    )
    .bind(service)
    .fetch_one(&mut **tx)
    .await?;
    if blocked {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Restore is incomplete. Delete this stopped target and restore into a new service.",
        ));
    }
    Ok(())
}
#[derive(Default, Deserialize)]
struct BackupConsent {
    #[serde(default)]
    offline: bool,
}
async fn list(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Vec<Record>>, ApiError> {
    access(&state, &id, service).await?;
    Ok(Json(sqlx::query_as("SELECT j.id,j.managed_database_id,j.managed_database_name,j.application_id,a.name AS service_name,e.project_id,j.source_backup_id,j.operation,j.engine,j.status,j.message,j.size_bytes,j.sha256,CASE WHEN j.s3_cipher IS NULL THEN 'local' ELSE 's3' END AS storage,j.scheduled_for,j.pruning,j.pruned_at,j.retention_error,j.created_at,j.started_at,j.finished_at FROM database_jobs j JOIN applications a ON a.id=j.application_id JOIN environments e ON e.id=a.environment_id WHERE j.application_id=$1 OR j.source_application_id=$1 ORDER BY j.created_at DESC,j.id DESC LIMIT 100")
        .bind(service).fetch_all(&state.pool).await?))
}
async fn create(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    input: Option<Json<BackupConsent>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    id.manager()?;
    let app = access(&state, &id, service).await?;
    if app.kind == "application" && !input.is_some_and(|Json(v)| v.offline) {
        return Err(invalid(
            "Confirm offline downtime before backing up an Application.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(missing)?;
    ensure_idle(&mut tx, service).await?;
    ensure_restored(&mut tx, service).await?;
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND status IN ('queued','running'))").bind(service).fetch_one(&mut *tx).await?;
    if pending {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Wait for the deployment to finish before backing up.",
        ));
    }
    let job = Uuid::new_v4();
    let inserted = sqlx::query("INSERT INTO database_jobs(id,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,deployment_id) SELECT $1,a.id,$2,'backup',CASE WHEN a.service_kind='application' THEN 'application' WHEN d.image LIKE 'mariadb:%' THEN 'mariadb' ELSE 'postgres' END,d.image,d.image_id,d.service_cipher,d.env_cipher,d.id FROM applications a JOIN deployments d ON d.id=a.active_deployment_id AND d.application_id=a.id WHERE a.id=$3 AND a.active_container_id IS NOT NULL AND d.image_id IS NOT NULL")
        .bind(job).bind(id.user_id).bind(service).execute(&mut *tx).await.map_err(database_error)?;
    if inserted.rows_affected() != 1 {
        return Err(invalid("Deploy this service before creating a backup."));
    }
    sqlx::query("UPDATE database_jobs SET application_port=(SELECT container_port FROM deployments WHERE id=database_jobs.deployment_id) WHERE id=$1").bind(job).execute(&mut *tx).await?;
    let (s3_cipher, s3_key) = policy::snapshot(&state, &mut tx, service, job).await?;
    sqlx::query("UPDATE database_jobs SET s3_cipher=$2,s3_key=$3 WHERE id=$1")
        .bind(job)
        .bind(s3_cipher)
        .bind(s3_key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id":job,"status":"queued"})),
    ))
}
#[derive(Deserialize)]
struct RestoreRequest {
    name: String,
}
/// Reuses the same worker, integrity checks, retention volume and S3 snapshot.
pub(super) async fn managed_job(
    state: &AppState,
    id: &Identity,
    service: Uuid,
    database: Uuid,
    name: &str,
    source: Option<Uuid>,
) -> Result<Value, ApiError> {
    id.manager()?;
    access(state, id, service).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .fetch_one(&mut *tx)
        .await?;
    project_bundles::lock_app(&mut tx, service, id.organization_id).await?;
    ensure_idle(&mut tx, service).await?;
    ensure_restored(&mut tx, service).await?;
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND status IN ('queued','running'))").bind(service).fetch_one(&mut *tx).await?;
    if pending {
        return Err(invalid("Wait for the shared MariaDB deployment to finish."));
    }
    let (image,image_id,cipher,env,deployment): (String,String,Vec<u8>,Option<Vec<u8>>,Uuid) = sqlx::query_as("SELECT d.image,d.image_id,d.service_cipher,d.env_cipher,d.id FROM applications a JOIN deployments d ON d.id=a.active_deployment_id WHERE a.id=$1 AND a.active_container_id IS NOT NULL AND d.image_id IS NOT NULL")
        .bind(service).fetch_optional(&mut *tx).await?.ok_or_else(||invalid("Deploy MariaDB before creating a logical database backup."))?;
    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_databases WHERE id=$1 AND service_id=$2 AND database_name=$3 AND status='ready')")
        .bind(database).bind(service).bind(name).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(invalid("Logical database is not ready."));
    }
    let mut config: DatabaseConfig = services::open(state, service, &cipher)?;
    config.database_name = name.to_owned();
    let cipher = services::seal(state, service, &config)?;
    let job = Uuid::new_v4();
    let (operation, s3_cipher, s3_key) = if let Some(backup) = source {
        let remote: Option<(Option<Vec<u8>>,Option<String>)> = sqlx::query_as("SELECT s3_cipher,s3_key FROM database_jobs WHERE id=$1 AND application_id=$2 AND managed_database_id=$3 AND operation='backup' AND status='succeeded' AND size_bytes>0 AND sha256 IS NOT NULL AND pruned_at IS NULL AND NOT pruning FOR UPDATE")
            .bind(backup).bind(service).bind(database).fetch_optional(&mut *tx).await?;
        let (cipher, key) = remote.ok_or_else(missing)?;
        ("restore", cipher, key)
    } else {
        let (cipher, key) = policy::snapshot(state, &mut tx, service, job).await?;
        ("backup", cipher, key)
    };
    sqlx::query("INSERT INTO database_jobs(id,application_id,source_application_id,source_backup_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,deployment_id,managed_database_id,managed_database_name,s3_cipher,s3_key) VALUES($1,$2,$3,$4,$5,$6,'mariadb',$7,$8,$9,$10,$11,$12,$13,$14,$15)")
        .bind(job).bind(service).bind(source.map(|_|service)).bind(source).bind(id.user_id).bind(operation).bind(image).bind(image_id).bind(cipher).bind(env).bind(deployment).bind(database).bind(name).bind(s3_cipher).bind(s3_key).execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await?;
    Ok(json!({"id":job,"status":"queued","database_name":name,"scope":"logical_database"}))
}
async fn restore(
    State(state): State<AppState>,
    id: Identity,
    Path((service, backup)): Path<(Uuid, Uuid)>,
    key: Option<axum::Extension<crate::auth::api_keys::KeyAccess>>,
    Json(input): Json<RestoreRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    id.manager()?;
    let source = access(&state, &id, service).await?;
    let label = name(&input.name)?;
    let mut tx = state.pool.begin().await?;
    let reserved = quotas::reserve(&state, &mut tx).await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(missing)?;
    ensure_idle(&mut tx, service).await?;
    let saved: Job = sqlx::query_as("SELECT id,application_id,source_application_id,source_backup_id,operation,engine,image,image_id,config_cipher,env_cipher,size_bytes,sha256,s3_cipher,s3_key,application_port FROM database_jobs WHERE id=$1 AND application_id=$2 AND operation='backup' AND status='succeeded' AND size_bytes>0 AND sha256 IS NOT NULL AND pruned_at IS NULL AND NOT pruning AND managed_database_id IS NULL FOR UPDATE")
        .bind(backup).bind(service).fetch_optional(&mut *tx).await?.ok_or_else(missing)?;
    let target = Uuid::new_v4();
    let deployment = Uuid::new_v4();
    let job = Uuid::new_v4();
    if saved.engine == "application" {
        return applications::queue_restore(
            &state,
            &id,
            &source,
            &saved,
            &label,
            tx,
            key.as_ref().map(|v| &v.0),
        )
        .await;
    }
    let mut config: DatabaseConfig = services::open(&state, service, &saved.config_cipher)?;
    config.password = services::random_password();
    if config.engine == DatabaseEngine::Mariadb {
        config.root_password = Some(services::random_password());
    }
    let config_cipher = services::seal(&state, target, &config)?;
    let values = decrypt(&state, service, saved.env_cipher.as_deref())?;
    let env_cipher = encrypt(&state, target, &values)?;
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,service_cipher,env_cipher) VALUES($1,$2,$3,$4,$5,'database',$6,$7)")
        .bind(target).bind(source.environment_id).bind(label).bind(&saved.image).bind(config.engine.port()).bind(&config_cipher).bind(&env_cipher).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port,env_cipher,service_kind,service_cipher,pinned_image_id,requested_by_name,requested_by_key_id,requested_by_key_name) VALUES($1,$2,$3,$4,$5,$6,'database',$7,$8,$9,$10,$11)")
        .bind(deployment).bind(target).bind(id.user_id).bind(&saved.image).bind(config.engine.port()).bind(&env_cipher).bind(&config_cipher).bind(&saved.image_id).bind(&id.name).bind(key.as_ref().map(|key|key.id)).bind(key.as_ref().map(|key|key.name.as_str())).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO database_jobs(id,application_id,source_application_id,source_backup_id,deployment_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher) VALUES($1,$2,$3,$4,$5,$6,'restore',$7,$8,$9,$10,$11)")
        .bind(job).bind(target).bind(service).bind(backup).bind(deployment).bind(id.user_id).bind(saved.engine).bind(saved.image).bind(saved.image_id).bind(config_cipher).bind(env_cipher).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE database_jobs SET s3_cipher=$2,s3_key=$3 WHERE id=$1")
        .bind(job)
        .bind(saved.s3_cipher)
        .bind(saved.s3_key)
        .execute(&mut *tx)
        .await?;
    quotas::check(&state, &mut tx, reserved).await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            json!({"id":job,"status":"queued","application_id":target,"project_id":source.project_id}),
        ),
    ))
}

fn helper_name(job: Uuid) -> String {
    format!("ferrix-backup-{}", job.simple())
}
fn volume_name(service: Uuid) -> String {
    format!("ferrix-backups-{}", service.simple())
}
fn owned(labels: Option<&HashMap<String, String>>, job: &Job) -> bool {
    labels.is_some_and(|labels| {
        labels.get("io.ferrix.backup-job") == Some(&job.id.to_string())
            && labels.get("io.ferrix.application") == Some(&job.application_id.to_string())
            && labels.get("io.ferrix.managed").map(String::as_str) == Some("true")
    })
}
async fn cleanup_helper(state: &AppState, job: &Job) -> Result<(), String> {
    for name in [helper_name(job.id), remote::transfer_name(job.id)] {
        cleanup_one(state, job, name).await?;
    }
    Ok(())
}
async fn cleanup_one(state: &AppState, job: &Job, name: String) -> Result<(), String> {
    match state.docker.inspect_container(&name, None).await {
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => return Ok(()),
        Ok(info) if owned(info.config.as_ref().and_then(|c| c.labels.as_ref()), job) => {}
        _ => return Err("Could not verify backup helper ownership for cleanup.".into()),
    }
    state
        .docker
        .remove_container(
            &name,
            Some(
                RemoveContainerOptionsBuilder::default()
                    .force(true)
                    .v(true)
                    .build(),
            ),
        )
        .await
        .map_err(|_| "Could not stop the backup helper. Recovery will retry cleanup.".to_owned())
}
async fn storage(state: &AppState, service: Uuid, create: bool) -> Result<String, String> {
    let name = volume_name(service);
    let info = match state.docker.inspect_volume(&name).await {
        Ok(info) => info,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) if create => state
            .docker
            .create_volume(VolumeCreateRequest {
                name: Some(name.clone()),
                labels: Some(HashMap::from([
                    ("io.ferrix.managed".into(), "true".into()),
                    ("io.ferrix.application".into(), service.to_string()),
                    ("io.ferrix.backups".into(), "true".into()),
                ])),
                ..Default::default()
            })
            .await
            .map_err(|_| "Could not create backup storage.")?,
        _ => return Err("Backup storage is unavailable on this Docker server.".into()),
    };
    if info.driver != "local"
        || !info.options.is_empty()
        || info.labels.get("io.ferrix.application") != Some(&service.to_string())
        || info.labels.get("io.ferrix.backups").map(String::as_str) != Some("true")
        || info.labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
    {
        return Err("Backup volume ownership could not be verified.".into());
    }
    Ok(name)
}
// Scripts contain no interpolated user input. Credentials and validated database
// identifiers are passed through quoted environment variables to native clients.
const PG_BACKUP: &str = r#"set -eu
umask 077
rm -f /backups/*.part
trap 'rm -f "$BACKUP_FILE.part"' EXIT
pg_dump --format=custom --no-owner --no-acl --file="$BACKUP_FILE.part" 2>/dev/null
mv "$BACKUP_FILE.part" "$BACKUP_FILE"
printf '%s %s\n' "$(wc -c < "$BACKUP_FILE")" "$(sha256sum "$BACKUP_FILE" | cut -d ' ' -f 1)"
"#;
const MARIA_BACKUP: &str = r#"set -eu
umask 077
rm -f /backups/*.part
trap 'rm -f "$BACKUP_FILE.part"' EXIT
mariadb-dump --host="$DB_HOST" --user=root --single-transaction --quick --routines --events --triggers --hex-blob --default-character-set=utf8mb4 "$DB_NAME" > "$BACKUP_FILE.part" 2>/dev/null
mv "$BACKUP_FILE.part" "$BACKUP_FILE"
printf '%s %s\n' "$(wc -c < "$BACKUP_FILE")" "$(sha256sum "$BACKUP_FILE" | cut -d ' ' -f 1)"
"#;
const PG_RESTORE: &str = r#"set -eu
if ! printf '%s  %s\n' "$BACKUP_SHA256" "$BACKUP_FILE" | sha256sum -c - >/dev/null 2>&1; then exit 42; fi
pg_restore --clean --if-exists --single-transaction --no-owner --no-acl --dbname="$PGDATABASE" "$BACKUP_FILE" >/dev/null 2>&1
"#;
const MARIA_RESTORE: &str = r#"set -eu
if ! printf '%s  %s\n' "$BACKUP_SHA256" "$BACKUP_FILE" | sha256sum -c - >/dev/null 2>&1; then exit 42; fi
mariadb --host="$DB_HOST" --user=root --default-character-set=utf8mb4 "$DB_NAME" < "$BACKUP_FILE" >/dev/null 2>&1
"#;
async fn execute(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Job,
) -> Result<(Option<i64>, Option<String>), String> {
    if job.engine == "application" {
        return applications::execute(state, conn, job).await;
    }
    let (container, environment, active_deployment): (Option<String>, Uuid, Option<Uuid>) = sqlx::query_as("SELECT active_container_id,environment_id,active_deployment_id FROM applications WHERE id=$1")
        .bind(job.application_id).fetch_one(&mut *conn).await.map_err(|_| "Could not read database runtime.")?;
    let container = container.ok_or("Database has no active container.")?;
    let info = worker::inspect_owned(state, job.application_id, &container)
        .await
        .map_err(|_| "Database container ownership could not be verified.")?;
    if !info.state.is_some_and(|s| s.running == Some(true)) {
        return Err("Start the database before backing it up or restoring.".into());
    }
    let expected: Option<Uuid> =
        sqlx::query_scalar("SELECT deployment_id FROM database_jobs WHERE id=$1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|_| "Could not verify the database version.")?;
    if active_deployment != expected {
        return Err("Database version changed before this operation started.".into());
    }
    let config: DatabaseConfig = services::open(state, job.application_id, &job.config_cipher)
        .map_err(|_| "Could not decrypt database credentials.")?;
    let restoring = job.operation == "restore";
    if let Some(database) = job.managed_database_id {
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_databases WHERE id=$1 AND service_id=$2 AND database_name=$3 AND status='ready')")
            .bind(database).bind(job.application_id).bind(&config.database_name).fetch_one(&mut *conn).await.map_err(|_| "Could not verify the managed database mapping.")?;
        if !valid {
            return Err(
                "Managed database mapping is no longer ready. No import was attempted.".into(),
            );
        }
    }
    let source = job.source_application_id.unwrap_or(job.application_id);
    let file_id = job.source_backup_id.unwrap_or(job.id);
    if restoring && job.s3_cipher.is_some() {
        sqlx::query("UPDATE database_jobs SET message='Downloading and verifying the original S3 backup.' WHERE id=$1").bind(job.id).execute(&mut *conn).await.map_err(|_|"Could not record download progress.")?;
        remote::download(state, conn, job, source, file_id).await?;
    }
    let volume = storage(state, source, !restoring).await?;
    let checksum: Option<String> = if restoring {
        sqlx::query_scalar("SELECT sha256 FROM database_jobs WHERE id=$1 AND application_id=$2 AND operation='backup' AND status='succeeded'")
            .bind(file_id).bind(source).fetch_optional(&mut *conn).await.map_err(|_| "Could not read backup integrity information.")?.flatten()
    } else {
        None
    };
    if restoring && checksum.is_none() {
        return Err("The source backup is no longer available.".into());
    }
    let network = services::database::network(environment);
    let net = state
        .docker
        .inspect_network(&network, None)
        .await
        .map_err(|_| "Database environment network is unavailable.")?;
    if net
        .labels
        .as_ref()
        .and_then(|l| l.get("io.ferrix.environment"))
        != Some(&environment.to_string())
    {
        return Err("Database network ownership could not be verified.".into());
    }
    let host = format!("ferrix-{}", job.application_id.simple());
    let env = match config.engine {
        DatabaseEngine::Postgres => vec![
            format!("PGHOST={host}"),
            format!("PGUSER={}", config.username),
            format!("PGPASSWORD={}", config.password),
            format!("PGDATABASE={}", config.database_name),
            "PGCONNECT_TIMEOUT=10".into(),
        ],
        DatabaseEngine::Mariadb => vec![
            format!("DB_HOST={host}"),
            format!("DB_NAME={}", config.database_name),
            format!(
                "MYSQL_PWD={}",
                config
                    .root_password
                    .ok_or("Missing MariaDB root credentials.")?
            ),
        ],
        _ => return Err("This engine does not support backups.".into()),
    };
    let script = match (config.engine, restoring) {
        (DatabaseEngine::Postgres, false) => PG_BACKUP,
        (DatabaseEngine::Postgres, true) => PG_RESTORE,
        (DatabaseEngine::Mariadb, false) => MARIA_BACKUP,
        (DatabaseEngine::Mariadb, true) => MARIA_RESTORE,
        _ => return Err("Unsupported database backup operation.".into()),
    };
    let mut env = env;
    env.push(format!("BACKUP_FILE=/backups/{}.dump", file_id.simple()));
    if let Some(checksum) = checksum {
        env.push(format!("BACKUP_SHA256={checksum}"));
    }
    let helper = helper_name(job.id);
    let message = if restoring {
        "Importing backup into the new database service."
    } else {
        "Creating a consistent database dump."
    };
    sqlx::query("UPDATE database_jobs SET message=$1 WHERE id=$2")
        .bind(message)
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not record backup progress.")?;
    let helper_config = ContainerCreateBody {
        image: Some(job.image_id.clone()),
        user: Some("0:0".into()),
        entrypoint: Some(vec!["sh".into(), "-c".into()]),
        cmd: Some(vec![script.into()]),
        env: Some(env),
        labels: Some(HashMap::from([
            ("io.ferrix.managed".into(), "true".into()),
            (
                "io.ferrix.application".into(),
                job.application_id.to_string(),
            ),
            ("io.ferrix.backup-job".into(), job.id.to_string()),
        ])),
        host_config: Some(HostConfig {
            network_mode: Some(network),
            readonly_rootfs: Some(true),
            memory: Some(256 * 1024 * 1024),
            nano_cpus: Some(1_000_000_000),
            pids_limit: Some(64),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            tmpfs: Some(HashMap::from([(
                "/tmp".into(),
                "rw,noexec,nosuid,size=67108864".into(),
            )])),
            mounts: Some(vec![Mount {
                typ: Some(MountTypeEnum::VOLUME),
                source: Some(volume),
                target: Some("/backups".into()),
                read_only: Some(restoring),
                ..Default::default()
            }]),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".into()),
                config: Some(HashMap::from([
                    ("max-size".into(), "1m".into()),
                    ("max-file".into(), "1".into()),
                ])),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    state.docker.create_container(Some(CreateContainerOptionsBuilder::default().name(&helper).build()), helper_config)
        .await.map_err(|_| "Could not create the database backup helper. Check the original image is still available.")?;
    state
        .docker
        .start_container(&helper, None)
        .await
        .map_err(|_| "Could not start the database backup helper.")?;
    loop {
        let info = state
            .docker
            .inspect_container(&helper, None)
            .await
            .map_err(|_| "Could not monitor database helper.")?;
        let status = info.state.ok_or("Database helper has no runtime status.")?;
        if status.running != Some(true) {
            if restoring && status.exit_code == Some(42) {
                return Err(
                    "Backup is missing or failed integrity verification. Choose another backup."
                        .into(),
                );
            }
            if status.exit_code != Some(0) {
                return Err("Database operation failed. Check free disk space, database connectivity and engine permissions.".into());
            }
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if restoring {
        return Ok((None, None));
    }
    let (size, hash) = helper_output(state, &helper).await?;
    if job.s3_cipher.is_some() {
        sqlx::query(
            "UPDATE database_jobs SET message='Uploading verified backup to S3.' WHERE id=$1",
        )
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not record upload progress.")?;
        remote::upload(state, job, size, &hash).await?;
    }
    Ok((Some(size), Some(hash)))
}
async fn helper_output(state: &AppState, helper: &str) -> Result<(i64, String), String> {
    let mut output = Vec::new();
    let mut logs = state.docker.logs(
        helper,
        Some(LogsOptionsBuilder::default().stdout(true).tail("5").build()),
    );
    while let Some(line) = logs.next().await {
        let line = line.map_err(|_| "Could not verify backup output.")?;
        let bytes = line.into_bytes();
        if output.len() + bytes.len() > 1024 {
            return Err("Unexpected backup helper output.".into());
        }
        output.extend_from_slice(&bytes);
    }
    let output = String::from_utf8(output).map_err(|_| "Invalid backup output.")?;
    let mut parts = output.split_whitespace();
    let size = parts
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|s| *s > 0)
        .ok_or("Backup file is empty or invalid.")?;
    let hash = parts
        .next()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Backup checksum is missing.")?
        .to_owned();
    Ok((size, hash))
}

pub(super) async fn cycle(
    state: &AppState,
    conn: &mut PgConnection,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<bool, String> {
    cycle_for(state, conn, shutdown, None).await
}
pub(in crate::deployments) async fn cycle_for(
    state: &AppState,
    conn: &mut PgConnection,
    shutdown: &mut watch::Receiver<bool>,
    only: Option<Uuid>,
) -> Result<bool, String> {
    let interrupted: Vec<Job> = sqlx::query_as(
        "SELECT * FROM database_jobs WHERE status='running' ORDER BY created_at LIMIT 10",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| "Could not recover database operations.")?;
    for job in interrupted {
        tokio::time::timeout(Duration::from_secs(15), cleanup_helper(state, &job))
            .await
            .map_err(|_| "Backup helper cleanup timed out.")??;
        if job.engine == "application" {
            applications::resume(state, conn, &job).await?;
            if job.operation == "backup" {
                remote::remove_local(state, &job).await?;
                cleanup_helper(state, &job).await?;
            }
        }
        let message = if job.operation == "restore" {
            "Restore interrupted. Helper stopped; completion is unconfirmed. Review the new service before retrying."
        } else {
            "Backup interrupted. Helper stopped. Create a new backup to retry."
        };
        if job.engine == "application" && job.operation == "backup" && job.s3_cipher.is_some() {
            sqlx::query("UPDATE database_jobs SET pruning=true,prune_retry_at=now(),retention_error='Interrupted upload cleanup pending.' WHERE id=$1").bind(job.id).execute(&mut *conn).await.map_err(|_|"Could not schedule interrupted upload cleanup.")?;
        }
        sqlx::query("UPDATE database_jobs SET status='failed',message=$2,finished_at=now() WHERE id=$1 AND status='running'")
            .bind(job.id).bind(message).execute(&mut *conn).await.map_err(|_| "Could not record interrupted database operation.")?;
    }
    policy::schedule_due(state, conn).await?;
    logical_policy::schedule_due(state, conn).await?;
    policy::prune_one(state, conn).await?;
    let job: Option<Job> = sqlx::query_as("WITH next AS (SELECT j.id FROM database_jobs j LEFT JOIN deployments d ON d.id=j.deployment_id WHERE j.status='queued' AND ($1::uuid IS NULL OR j.id=$1) AND (j.operation='backup' OR d.status IN ('succeeded','failed') OR d.id IS NULL) ORDER BY j.created_at,j.id LIMIT 1 FOR UPDATE OF j SKIP LOCKED) UPDATE database_jobs j SET status='running',started_at=now() FROM next WHERE j.id=next.id RETURNING j.*")
        .bind(only)
        .fetch_optional(&mut *conn).await.map_err(|_| "Could not claim database operation.")?;
    let Some(job) = job else {
        return Ok(false);
    };
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM database_jobs j JOIN applications a ON a.id=j.application_id JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id JOIN auth_users u ON u.id=j.requested_by JOIN auth_members m ON m.user_id=u.id AND m.organization_id=p.organization_id WHERE j.id=$1 AND NOT auth_user_banned(u.banned,u.ban_expires_at) AND m.role IN ('owner','admin'))")
        .bind(job.id).fetch_one(&mut *conn).await.map_err(|_| "Could not recheck database operation permissions.")?;
    let result = if allowed {
        tokio::select! {
            result=tokio::time::timeout(Duration::from_secs(1200),execute(state,conn,&job)) => result.unwrap_or_else(|_| Err("Database operation timed out after twenty minutes.".into())),
            _=shutdown.changed() => Err("Database operation interrupted by server shutdown.".into()),
        }
    } else {
        Err("The requesting user no longer has permission for this database operation.".into())
    };
    // Keep the job running until the helper is confirmed stopped. Runtime controls
    // and deletion remain blocked if Docker cleanup needs recovery on a later cycle.
    tokio::time::timeout(Duration::from_secs(15), cleanup_helper(state, &job))
        .await
        .map_err(|_| "Database helper cleanup timed out.")??;
    if job.engine == "application" {
        applications::resume(state, conn, &job).await?;
    }
    if result.is_err()
        && job.operation == "backup"
        && (job.s3_cipher.is_some() || job.engine == "application")
    {
        tokio::time::timeout(Duration::from_secs(15), remote::remove_local(state, &job))
            .await
            .map_err(|_| "Failed backup file cleanup timed out.")??;
        tokio::time::timeout(Duration::from_secs(15), cleanup_helper(state, &job))
            .await
            .map_err(|_| "Failed transfer helper cleanup timed out.")??;
    }
    let (status, message, size, hash) = match result {
        Ok((size, hash)) => (
            "succeeded",
            if job.operation == "backup" {
                if job.s3_cipher.is_some() {
                    "Backup saved locally and on S3.".to_owned()
                } else {
                    "Backup completed.".to_owned()
                }
            } else {
                if job.managed_database_id.is_some() {
                    "Logical database restored in place. Sibling databases and SQL accounts were not restored or rotated.".to_owned()
                } else if job.engine == "application" {
                    "Restore completed. The new Application is stopped. Verify its data and stop the source before starting the target.".to_owned()
                } else {
                    "Restore completed. The new database service is ready.".to_owned()
                }
            },
            size,
            hash,
        ),
        Err(message) => ("failed", message, None, None),
    };
    use sqlx::Connection;
    let mut complete = conn
        .begin()
        .await
        .map_err(|_| "Could not start backup completion.")?;
    sqlx::query("UPDATE database_jobs SET status=$1,message=$2,size_bytes=$3,sha256=$4,finished_at=now() WHERE id=$5 AND status='running'")
        .bind(status).bind(message).bind(size).bind(hash).bind(job.id).execute(&mut *complete).await.map_err(|_| "Could not persist database operation completion.")?;
    if status == "succeeded" && job.operation == "restore" && job.engine == "application" {
        sqlx::query("UPDATE applications SET restore_blocked=false WHERE id=$1")
            .bind(job.application_id)
            .execute(&mut *complete)
            .await
            .map_err(|_| "Could not unblock restored Application.")?;
    }
    complete
        .commit()
        .await
        .map_err(|_| "Could not confirm backup completion.")?;
    if status == "failed"
        && job.engine == "application"
        && job.operation == "backup"
        && job.s3_cipher.is_some()
    {
        sqlx::query("UPDATE database_jobs SET pruning=true,prune_retry_at=now(),retention_error='Failed upload cleanup pending.' WHERE id=$1").bind(job.id).execute(&mut *conn).await.map_err(|_|"Could not schedule failed upload cleanup.")?;
    }
    if status == "succeeded" && job.operation == "backup" {
        policy::mark_excess(conn, job.application_id).await?;
    }
    Ok(true)
}

pub(super) async fn bundle_read(state: &AppState, id: Uuid) -> Result<Vec<u8>, String> {
    let job: Job = sqlx::query_as("SELECT * FROM database_jobs WHERE id=$1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| "Could not load project backup member.")?;
    remote::bundle_read(state, &job).await
}
pub(super) async fn bundle_write(state: &AppState, id: Uuid, bytes: Vec<u8>) -> Result<(), String> {
    let job: Job = sqlx::query_as("SELECT * FROM database_jobs WHERE id=$1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| "Could not load imported backup member.")?;
    remote::bundle_write(state, &job, bytes).await
}

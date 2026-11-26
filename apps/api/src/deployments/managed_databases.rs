//! One private MariaDB server, independently authorized application accounts.
use super::*;
use bollard::exec::{CreateExecOptions, StartExecResults};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

#[derive(sqlx::FromRow, Serialize)]
struct Record {
    id: Uuid,
    service_id: Uuid,
    application_id: Option<Uuid>,
    database_name: String,
    username: String,
    status: String,
    max_connections: i32,
    message: String,
    #[serde(skip)]
    credentials_cipher: Vec<u8>,
    #[serde(skip)]
    pending_credentials_cipher: Option<Vec<u8>>,
}
#[derive(Serialize, Deserialize)]
struct Secret {
    password: String,
    ownership: String,
}
#[derive(Deserialize)]
struct Provision {
    application_id: Uuid,
}
#[derive(Deserialize)]
struct Confirm {
    database_name: String,
}
pub(super) async fn provision_application(
    state: &AppState,
    id: &Identity,
    service: Uuid,
    app: Uuid,
) -> Result<(), ApiError> {
    let Json(r) = provision(
        State(state.clone()),
        id.clone(),
        Path(service),
        Json(Provision {
            application_id: app,
        }),
    )
    .await?;
    inject(
        State(state.clone()),
        id.clone(),
        Path((service, r.id)),
        Json(Confirm {
            database_name: r.database_name,
        }),
    )
    .await?;
    Ok(())
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/databases", get(list))
        .route("/services/{id}/database-connection", get(connection))
        .route(
            "/services/{id}/databases/{database}/environment",
            get(preview),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ))
        .merge(
            Router::new()
                .route(
                    "/services/{id}/databases/{database}",
                    axum::routing::delete(remove),
                )
                .route("/services/{id}/databases/{database}/rotate", post(rotate))
                .route("/services/{id}/databases/{database}/backups", post(backup))
                .route(
                    "/services/{id}/databases/{database}/backups/{backup}/restore",
                    post(restore_backup),
                )
                .route("/services/{id}/databases", post(provision))
                .route("/services/{id}/databases/{database}/retry", post(retry))
                .route(
                    "/services/{id}/databases/{database}/environment",
                    post(inject),
                )
                .route("/services/{id}/databases/{database}/revoke", post(revoke))
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
}
async fn backup(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let r = record(&state, &id, service, database).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(backups::managed_job(&state, &id, service, database, &r.database_name, None).await?),
    ))
}
async fn restore_backup(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database, backup)): Path<(Uuid, Uuid, Uuid)>,
    Json(input): Json<Confirm>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let r = record(&state, &id, service, database).await?;
    if input.database_name != r.database_name {
        return Err(invalid(
            "Confirm the exact logical database name. Restore replaces tables in this database only.",
        ));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(
            backups::managed_job(
                &state,
                &id,
                service,
                database,
                &r.database_name,
                Some(backup),
            )
            .await?,
        ),
    ))
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "MariaDB operation failed. Credentials were retained for a safe retry; check the shared service is running.",
    )
}
fn collision() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "Database or account ownership could not be verified. Existing data and accounts were not adopted or deleted.",
    )
}
async fn record(
    state: &AppState,
    id: &Identity,
    service: Uuid,
    database: Uuid,
) -> Result<Record, ApiError> {
    id.manager()?;
    app_access(state, id, service).await?;
    sqlx::query_as(
        "SELECT * FROM managed_databases WHERE id=$1 AND service_id=$2 AND organization_id=$3",
    )
    .bind(database)
    .bind(service)
    .bind(id.organization_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(missing)
}
async fn list(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Vec<Record>>, ApiError> {
    id.manager()?;
    app_access(&state, &id, service).await?;
    Ok(Json(sqlx::query_as("SELECT * FROM managed_databases WHERE service_id=$1 AND organization_id=$2 ORDER BY created_at")
        .bind(service).bind(id.organization_id).fetch_all(&state.pool).await?))
}
async fn server(state: &AppState, id: &Identity, service: Uuid) -> Result<Application, ApiError> {
    let app = app_access(state, id, service).await?;
    if app.database_engine.as_deref() != Some("mariadb") || app.kind != "database" {
        return Err(invalid("Choose a MariaDB database service."));
    }
    Ok(app)
}
// SQL never appears in Docker's command arguments, API responses, or application logs.
// Both names and secrets are generated ASCII; callers cannot supply SQL fragments.
async fn sql(
    state: &AppState,
    service: &Application,
    statement: String,
) -> Result<String, ApiError> {
    let container = service
        .active_container_id
        .as_deref()
        .ok_or_else(unavailable)?;
    let info = worker::inspect_owned(state, service.id, container).await?;
    if !info.state.as_ref().is_some_and(|s| s.running == Some(true)) {
        return Err(unavailable());
    }
    let work = async {
        let exec = state.docker.create_exec(container, CreateExecOptions {
            attach_stdin: Some(true), attach_stdout: Some(true), attach_stderr: Some(true),
            cmd: Some(vec!["/bin/sh", "-c", "MYSQL_PWD=\"$MARIADB_ROOT_PASSWORD\" exec mariadb --protocol=socket --user=root --batch --skip-column-names --skip-reconnect"].into_iter().map(str::to_owned).collect()),
            ..Default::default()
        }).await.map_err(|_| unavailable())?;
        let StartExecResults::Attached {
            mut input,
            mut output,
        } = state
            .docker
            .start_exec(&exec.id, None)
            .await
            .map_err(|_| unavailable())?
        else {
            return Err(unavailable());
        };
        input
            .write_all(statement.as_bytes())
            .await
            .map_err(|_| unavailable())?;
        input.shutdown().await.map_err(|_| unavailable())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = output.next().await {
            bytes.extend_from_slice(&chunk.map_err(|_| unavailable())?.into_bytes());
            if bytes.len() > 65536 {
                return Err(unavailable());
            }
        }
        if state
            .docker
            .inspect_exec(&exec.id)
            .await
            .map_err(|_| unavailable())?
            .exit_code
            != Some(0)
        {
            return Err(unavailable());
        }
        String::from_utf8(bytes).map_err(|_| unavailable())
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), work)
        .await
        .map_err(|_| unavailable())?
}
fn safe(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric())
}
async fn ownership(
    state: &AppState,
    service: &Application,
    r: &Record,
    secret: &Secret,
) -> Result<(bool, bool), ApiError> {
    if ![
        r.database_name.as_str(),
        r.username.as_str(),
        secret.password.as_str(),
        secret.ownership.as_str(),
    ]
    .into_iter()
    .all(safe)
    {
        return Err(internal());
    }
    let result = sql(state, service, format!("SELECT CONCAT('D:',COALESCE((SELECT SCHEMA_COMMENT FROM information_schema.SCHEMATA WHERE SCHEMA_NAME='{}'),'missing')); SELECT CONCAT('U:',COALESCE((SELECT IF(JSON_UNQUOTE(JSON_EXTRACT(Priv,'$.authentication_string'))=PASSWORD('{}') AND COALESCE(JSON_VALUE(Priv,'$.access'),0)=0,'owned','collision') FROM mysql.global_priv WHERE User='{}' AND Host='%'),'missing'));\n", r.database_name, secret.password, r.username)).await?;
    let database = result
        .lines()
        .find_map(|s| s.strip_prefix("D:"))
        .ok_or_else(unavailable)?;
    let user = result
        .lines()
        .find_map(|s| s.strip_prefix("U:"))
        .ok_or_else(unavailable)?;
    if database != "missing" && database != secret.ownership || !matches!(user, "missing" | "owned")
    {
        return Err(collision());
    }
    Ok((database != "missing", user != "missing"))
}
async fn run(state: &AppState, id: &Identity, database: Uuid) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    let r: Record = sqlx::query_as(
        "SELECT * FROM managed_databases WHERE id=$1 AND organization_id=$2 FOR UPDATE",
    )
    .bind(database)
    .bind(id.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(missing)?;
    if r.status == "ready" {
        return Ok(());
    }
    if matches!(r.status.as_str(), "revoked" | "rotating" | "deleting") {
        return Err(invalid("This account was explicitly revoked."));
    }
    let service = server(state, id, r.service_id).await?;
    let secret: Secret = services::open(state, r.id, &r.credentials_cipher)?;
    let result = async {
        let (has_database, has_user) = ownership(state, &service, &r, &secret).await?;
        if !has_database {
            sql(state, &service, format!("CREATE DATABASE `{}` CHARACTER SET utf8mb4 COMMENT '{}';\n",r.database_name,secret.ownership)).await?;
        }
        if !has_user {
            sql(state, &service, format!("CREATE USER '{}'@'%' IDENTIFIED BY '{}' WITH MAX_USER_CONNECTIONS {};\n",r.username,secret.password,r.max_connections)).await?;
        }
        sql(state, &service, format!("GRANT SELECT,INSERT,UPDATE,DELETE,CREATE,DROP,ALTER,INDEX,REFERENCES,CREATE TEMPORARY TABLES,LOCK TABLES,SHOW VIEW,CREATE VIEW,TRIGGER ON `{}`.* TO '{}'@'%';\n",r.database_name,r.username)).await?;
        Ok::<_,ApiError>(())
    }.await;
    let (status, message) = if result.is_ok() {
        (
            "ready",
            "Database ready. Preview and apply connection settings, then redeploy the linked application.",
        )
    } else {
        (
            "failed",
            "Provisioning incomplete. Fix MariaDB availability or inspect a naming collision, then retry. Existing data is retained.",
        )
    };
    sqlx::query("UPDATE managed_databases SET status=$2,message=$3,updated_at=now() WHERE id=$1")
        .bind(database)
        .bind(status)
        .bind(message)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    result
}
async fn rotate(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
    Json(input): Json<Confirm>,
) -> Result<StatusCode, ApiError> {
    let r = record(&state, &id, service, database).await?;
    if input.database_name != r.database_name {
        return Err(invalid(
            "Confirm the exact database name. Rotation invalidates this application's password; apply settings and redeploy afterwards.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    let r: Record = sqlx::query_as("SELECT * FROM managed_databases WHERE id=$1 FOR UPDATE")
        .bind(database)
        .fetch_one(&mut *tx)
        .await?;
    if !matches!(r.status.as_str(), "ready" | "rotating") {
        return Err(invalid(
            "Only a ready account or an interrupted rotation can be rotated.",
        ));
    }
    if r.pending_credentials_cipher.is_none() {
        let old: Secret = services::open(&state, r.id, &r.credentials_cipher)?;
        let new = Secret {
            password: services::random_password(),
            ownership: old.ownership,
        };
        let cipher = services::seal(&state, r.id, &new)?;
        sqlx::query("UPDATE managed_databases SET pending_credentials_cipher=$2,status='rotating',message='Password rotation pending. Retry rotation if interrupted. Apply new settings and redeploy the Application afterwards.',updated_at=now() WHERE id=$1").bind(database).bind(cipher).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    // Persist the new secret before changing SQL, so a process interruption is retryable.
    let mut tx = state.pool.begin().await?;
    let r: Record = sqlx::query_as("SELECT * FROM managed_databases WHERE id=$1 FOR UPDATE")
        .bind(database)
        .fetch_one(&mut *tx)
        .await?;
    let server = server(&state, &id, service).await?;
    let old: Secret = services::open(&state, r.id, &r.credentials_cipher)?;
    let cipher = r.pending_credentials_cipher.as_ref().ok_or_else(internal)?;
    let new: Secret = services::open(&state, r.id, cipher)?;
    let owned = match ownership(&state, &server, &r, &old).await {
        Ok(owned) => owned,
        Err(_) => ownership(&state, &server, &r, &new).await?,
    };
    if owned != (true, true) {
        return Err(collision());
    }
    sql(
        &state,
        &server,
        format!(
            "ALTER USER '{}'@'%' IDENTIFIED BY '{}';\n",
            r.username, new.password
        ),
    )
    .await?;
    sqlx::query("UPDATE managed_databases SET credentials_cipher=pending_credentials_cipher,pending_credentials_cipher=NULL,status='ready',message='Password rotated. Preview and apply the new connection settings, then redeploy the linked Application.',updated_at=now() WHERE id=$1").bind(database).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn provision(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<Provision>,
) -> Result<Json<Record>, ApiError> {
    id.manager()?;
    let db = server(&state, &id, service).await?;
    let app = app_access(&state, &id, input.application_id).await?;
    if app.kind != "application" || db.environment_id != app.environment_id {
        return Err(invalid(
            "Choose an Application in the same environment as MariaDB.",
        ));
    }
    let database = Uuid::new_v4();
    let secret = Secret {
        password: services::random_password(),
        ownership: services::random_password(),
    };
    let cipher = services::seal(&state, database, &secret)?;
    let mut tx = state.pool.begin().await?;
    // Serialize against service removal, movement and deployment snapshot creation.
    sqlx::query("SELECT id FROM applications WHERE id=ANY($1) ORDER BY id FOR UPDATE")
        .bind(vec![service, input.application_id])
        .fetch_all(&mut *tx)
        .await?;
    let bound: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id,service_id FROM managed_databases WHERE application_id=$1")
            .bind(input.application_id)
            .fetch_optional(&mut *tx)
            .await?;
    let database = if let Some((existing, existing_service)) = bound {
        if existing_service != service {
            return Err(invalid(
                "This Application already has a managed database on another service.",
            ));
        }
        existing
    } else {
        sqlx::query("INSERT INTO managed_databases(id,organization_id,service_id,application_id,database_name,username,credentials_cipher) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(database).bind(id.organization_id).bind(service).bind(input.application_id).bind(format!("fx{}",database.simple())).bind(format!("fu{}",&database.simple().to_string()[..30])).bind(cipher).execute(&mut *tx).await.map_err(database_error)?;
        database
    };
    tx.commit().await?;
    run(&state, &id, database).await?;
    Ok(Json(record(&state, &id, service, database).await?))
}
async fn retry(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
) -> Result<Json<Record>, ApiError> {
    record(&state, &id, service, database).await?;
    run(&state, &id, database).await?;
    Ok(Json(record(&state, &id, service, database).await?))
}
fn settings(state: &AppState, r: &Record) -> Result<Vec<String>, ApiError> {
    if r.status != "ready" {
        return Err(invalid(
            "The database account is not ready or has been revoked.",
        ));
    }
    let secret: Secret = services::open(state, r.id, &r.credentials_cipher)?;
    Ok(vec![
        format!("DB_HOST=ferrix-{}", r.service_id.simple()),
        "DB_PORT=3306".into(),
        format!("DB_DATABASE={}", r.database_name),
        format!("DB_USERNAME={}", r.username),
        format!("DB_PASSWORD={}", secret.password),
    ])
}
async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
    Json(input): Json<Confirm>,
) -> Result<StatusCode, ApiError> {
    record(&state, &id, service, database).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .fetch_one(&mut *tx)
        .await?;
    project_bundles::lock_app(&mut tx, service, id.organization_id).await?;
    backups::ensure_idle(&mut tx, service).await?;
    let r: Record = sqlx::query_as("SELECT * FROM managed_databases WHERE id=$1 FOR UPDATE")
        .bind(database)
        .fetch_one(&mut *tx)
        .await?;
    if input.database_name != r.database_name {
        return Err(invalid(
            "Type the exact logical database name to permanently delete its data and SQL account.",
        ));
    }
    if r.status == "rotating" {
        return Err(invalid(
            "Finish password rotation before deleting this database.",
        ));
    }
    let history: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM database_jobs WHERE managed_database_id=$1)",
    )
    .bind(database)
    .fetch_one(&mut *tx)
    .await?;
    if history {
        return Err(invalid(
            "This database has backup history. Retain its mapping and shared service so its backups remain recoverable.",
        ));
    }
    sqlx::query("UPDATE managed_databases SET status='deleting',message='Explicit data deletion pending. Retry deletion if interrupted.',updated_at=now() WHERE id=$1").bind(database).execute(&mut *tx).await?;
    tx.commit().await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .fetch_one(&mut *tx)
        .await?;
    let r: Record = sqlx::query_as("SELECT * FROM managed_databases WHERE id=$1 FOR UPDATE")
        .bind(database)
        .fetch_one(&mut *tx)
        .await?;
    let server = server(&state, &id, service).await?;
    let secret: Secret = services::open(&state, r.id, &r.credentials_cipher)?;
    let (has_database, has_user) = ownership(&state, &server, &r, &secret).await?;
    if has_database {
        sql(
            &state,
            &server,
            format!("DROP DATABASE `{}`;\n", r.database_name),
        )
        .await?;
    }
    if has_user {
        sql(
            &state,
            &server,
            format!("DROP USER '{}'@'%';\n", r.username),
        )
        .await?;
    }
    sqlx::query("DELETE FROM managed_databases WHERE id=$1")
        .bind(database)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn connection(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::permissions::developer(&state, &id, app).await?;
    let r: Record = sqlx::query_as(
        "SELECT * FROM managed_databases WHERE application_id=$1 AND organization_id=$2",
    )
    .bind(app)
    .bind(id.organization_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(missing)?;
    Ok(Json(
        json!({"database":r.database_name,"environment":settings(&state,&r)?.join("\n")}),
    ))
}
fn merge(existing: Vec<String>, managed: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = existing
        .into_iter()
        .filter(|line| {
            !managed.iter().any(|new| {
                new.split_once('=')
                    .map(|(key, _)| line.starts_with(&format!("{key}=")))
                    .unwrap_or(false)
            })
        })
        .collect();
    result.extend(managed);
    result
}
async fn preview(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let r = record(&state, &id, service, database).await?;
    let app = r
        .application_id
        .ok_or_else(|| invalid("Linked Application was removed. Database data is retained."))?;
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT env_cipher FROM applications WHERE id=$1")
            .bind(app)
            .fetch_one(&state.pool)
            .await?;
    let before = decrypt(&state, app, cipher.as_deref())?;
    let after = merge(before.clone(), settings(&state, &r)?);
    parse_environment(&after.join("\n"))?;
    Ok(Json(
        json!({"application_id":app,"before":before.join("\n"),"after":after.join("\n"),"requires_redeploy":true}),
    ))
}
async fn inject(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
    Json(input): Json<Confirm>,
) -> Result<StatusCode, ApiError> {
    let r = record(&state, &id, service, database).await?;
    if input.database_name != r.database_name {
        return Err(invalid("Confirm the exact logical database name."));
    }
    let app = r.application_id.ok_or_else(missing)?;
    let mut tx = state.pool.begin().await?;
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT env_cipher FROM applications WHERE id=$1 FOR UPDATE")
            .bind(app)
            .fetch_one(&mut *tx)
            .await?;
    backups::ensure_idle(&mut tx, app).await?;
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND status IN ('queued','running'))").bind(app).fetch_one(&mut *tx).await?;
    if pending {
        return Err(invalid(
            "Wait for the Application deployment to finish before updating connection settings.",
        ));
    }
    let values = merge(
        decrypt(&state, app, cipher.as_deref())?,
        settings(&state, &r)?,
    );
    parse_environment(&values.join("\n"))?;
    let encrypted = encrypt(&state, app, &values)?;
    sqlx::query("UPDATE applications SET env_cipher=$2 WHERE id=$1")
        .bind(app)
        .bind(encrypted)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn revoke(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
    Json(input): Json<Confirm>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    let r = record(&state, &id, service, database).await?;
    sqlx::query("SELECT id FROM managed_databases WHERE id=$1 FOR UPDATE")
        .bind(database)
        .fetch_one(&mut *tx)
        .await?;
    if input.database_name != r.database_name {
        return Err(invalid(
            "Confirm the exact logical database name. Revocation disconnects this application's credentials.",
        ));
    }
    let server = server(&state, &id, service).await?;
    let secret: Secret = services::open(&state, r.id, &r.credentials_cipher)?;
    ownership(&state, &server, &r, &secret).await?;
    sql(
        &state,
        &server,
        format!("ALTER USER '{}'@'%' ACCOUNT LOCK;\n", r.username),
    )
    .await?;
    sqlx::query("UPDATE managed_databases SET status='revoked',message='SQL account locked. Existing sessions may remain connected; data retained.',updated_at=now() WHERE id=$1").bind(database).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_names_and_environment_merge() {
        assert!(safe("fx012345abcdef"));
        for bad in ["", "class_a", "a%b", "a'b", "a;b", "a`b"] {
            assert!(!safe(bad));
        }
        assert_eq!(
            merge(
                vec![
                    "OTHER=keep".into(),
                    "DB_HOST=old".into(),
                    "DB_PASSWORD=old".into()
                ],
                vec!["DB_HOST=new".into(), "DB_PASSWORD=new".into()]
            ),
            vec!["OTHER=keep", "DB_HOST=new", "DB_PASSWORD=new"]
        );
    }
}

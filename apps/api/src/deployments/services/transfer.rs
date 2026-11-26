use super::*;
use bollard::query_parameters::RemoveContainerOptionsBuilder;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    environment_id: Uuid,
    name: Option<String>,
}
#[derive(sqlx::FromRow)]
struct Snapshot {
    env_cipher: Option<Vec<u8>>,
    service_cipher: Option<Vec<u8>>,
    source_upload_id: Option<Uuid>,
    notes: String,
}
fn conflict(message: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, message)
}
// Serialize with deployment/configuration writes; both operations stay in one project.
async fn locked(
    id: &Identity,
    service: Uuid,
    target: Uuid,
    conn: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Application, ApiError> {
    let app = sqlx::query_as::<_,Application>(&format!("SELECT {APP_COLUMNS} FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2 AND a.service_kind IN ('application','database') FOR UPDATE OF a"))
        .bind(service).bind(id.organization_id).fetch_optional(&mut **conn).await?.ok_or_else(missing)?;
    project_bundles::lock_app(conn, service, id.organization_id).await?;
    let linked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM managed_databases WHERE service_id=$1 OR application_id=$1) OR EXISTS(SELECT 1 FROM php_classroom_applications WHERE application_id=$1 OR database_service_id=$1)",
    )
    .bind(service)
    .fetch_one(&mut **conn)
    .await?;
    if linked {
        return Err(conflict(
            "Managed database links must stay in their original environment. Moving or cloning would copy credentials without creating an independent database.",
        ));
    }
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM environments WHERE id=$1 AND project_id=$2)",
    )
    .bind(target)
    .bind(app.project_id)
    .fetch_one(&mut **conn)
    .await?;
    if !valid {
        return Err(missing());
    }
    if target == app.environment_id {
        return Err(invalid("Choose a different environment in this project."));
    }
    super::super::backups::ensure_idle(conn, service).await?;
    super::super::backups::ensure_restored(conn, service).await?;
    if app.restore_blocked || app.cutover_pending || !app.traffic_enabled {
        return Err(conflict(
            "Finish the restore and project cutover before moving or cloning this service.",
        ));
    }
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND (status IN ('queued','running') OR restore_container_id IS NOT NULL)) OR EXISTS(SELECT 1 FROM application_git_sources WHERE application_id=$1 AND pending_delivery IS NOT NULL)")
        .bind(service).fetch_one(&mut **conn).await?;
    if pending {
        return Err(conflict(
            "Wait for pending deployments and GitHub deliveries to finish.",
        ));
    }
    // Keep target from being deleted during the operation.
    sqlx::query("SELECT id FROM environments WHERE id=$1 FOR KEY SHARE")
        .bind(target)
        .execute(&mut **conn)
        .await?;
    Ok(app)
}
pub(super) async fn move_service(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<Target>,
) -> Result<Json<Application>, ApiError> {
    id.manager()?;
    let mut tx = state.pool.begin().await?;
    let reserved = quotas::reserve(&state, &mut tx).await?;
    let app = locked(&id, service, input.environment_id, &mut tx).await?;
    if input.name.is_some() {
        return Err(invalid("Move keeps the existing service name."));
    }
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM applications WHERE environment_id=$1 AND name=$2)",
    )
    .bind(input.environment_id)
    .bind(&app.name)
    .fetch_one(&mut *tx)
    .await?;
    if duplicate {
        return Err(conflict(
            "A service with this name already exists in the destination.",
        ));
    }
    if let Some(container) = &app.active_container_id {
        let info = super::super::worker::inspect_owned(&state, service, container).await?;
        if !matches!(
            info.state.and_then(|s| s.status),
            Some(
                bollard::models::ContainerStateStatusEnum::EXITED
                    | bollard::models::ContainerStateStatusEnum::CREATED
            )
        ) {
            return Err(conflict(
                "Stop this service before moving it. Deploy it again in the destination environment.",
            ));
        }
    }
    // Update/validate first, so name and maintenance failures cannot remove a container.
    sqlx::query("UPDATE applications SET environment_id=$2,active_container_id=NULL,active_deployment_id=NULL,host_port=NULL,updated_at=now() WHERE id=$1")
        .bind(service).bind(input.environment_id).execute(&mut *tx).await.map_err(database_error)?;
    quotas::check(&state, &mut tx, reserved).await?;
    if let Some(container) = &app.active_container_id {
        tokio::time::timeout(
            Duration::from_secs(15),
            state.docker.remove_container(
                container,
                Some(
                    RemoveContainerOptionsBuilder::default()
                        .force(false)
                        .v(false)
                        .build(),
                ),
            ),
        )
        .await
        .map_err(|_| conflict("Could not remove the stopped container. Check Docker and retry."))?
        .map_err(|_| conflict("Could not remove the stopped container. Check Docker and retry."))?;
    }
    tx.commit().await?;
    Ok(Json(app_access(&state, &id, service).await?))
}
pub(super) async fn clone_service(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<Target>,
) -> Result<(StatusCode, Json<Application>), ApiError> {
    id.manager()?;
    let mut tx = state.pool.begin().await?;
    let reserved = quotas::reserve(&state, &mut tx).await?;
    let app = locked(&id, service, input.environment_id, &mut tx).await?;
    let label = name(input.name.as_deref().unwrap_or(&app.name))?;
    let snapshot: Snapshot = sqlx::query_as(
        "SELECT env_cipher,service_cipher,source_upload_id,notes FROM applications WHERE id=$1",
    )
    .bind(service)
    .fetch_one(&mut *tx)
    .await?;
    let new = Uuid::new_v4();
    let values = decrypt(&state, service, snapshot.env_cipher.as_deref())?;
    let env = encrypt(&state, new, &values)?;
    let config = if app.kind == "application" {
        let runtime =
            application_runtime::decode(&state, service, snapshot.service_cipher.as_deref())?;
        seal(&state, new, &runtime)?
    } else {
        let mut db: DatabaseConfig = open(
            &state,
            service,
            snapshot.service_cipher.as_deref().ok_or_else(internal)?,
        )?;
        db.password = random_password();
        if db.root_password.is_some() {
            db.root_password = Some(random_password());
        }
        seal(&state, new, &db)?
    };
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,env_cipher,service_cipher,worker_mode,notes) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(new).bind(input.environment_id).bind(label).bind(&app.image).bind(app.container_port).bind(&app.kind).bind(env).bind(config).bind(app.worker_mode).bind(snapshot.notes).execute(&mut *tx).await.map_err(database_error)?;
    super::super::github::clone_source(&state, &mut tx, service, new, id.user_id).await?;
    if let Some(upload) = snapshot.source_upload_id {
        let upload_id = Uuid::new_v4();
        let inserted = sqlx::query("INSERT INTO application_source_uploads(id,application_id,created_by,archive,sha256,context,dockerfile,preview) SELECT $1,$2,$3,archive,sha256,context,dockerfile,preview FROM application_source_uploads WHERE id=$4 AND application_id=$5")
            .bind(upload_id).bind(new).bind(id.user_id).bind(upload).bind(service).execute(&mut *tx).await?.rows_affected();
        if inserted != 1 {
            return Err(conflict(
                "Uploaded source is unavailable. Upload it again before cloning.",
            ));
        }
        sqlx::query("UPDATE applications SET source_upload_id=$2 WHERE id=$1")
            .bind(new)
            .bind(upload_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    }
    quotas::check(&state, &mut tx, reserved).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(app_access(&state, &id, new).await?),
    ))
}

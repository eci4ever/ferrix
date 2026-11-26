//! Coordinated project recovery. Installation auth and provider credentials never enter exports.
use super::*;
use axum::{
    body::Bytes,
    http::{HeaderMap, header},
    response::IntoResponse,
};
use bollard::models::{ContainerUpdateBody, RestartPolicy, RestartPolicyNameEnum};
use bollard::query_parameters::StopContainerOptionsBuilder;
use sqlx::{Acquire, PgConnection};
mod format;
mod operations;
use format::{Manifest, Service};
#[derive(sqlx::FromRow)]
struct Bundle {
    id: Uuid,
    organization_id: Uuid,
    project_id: Option<Uuid>,
    target_project_id: Option<Uuid>,
    requested_by: Option<Uuid>,
    operation: String,
    status: String,
    project_name: String,
    key_cipher: Option<Vec<u8>>,
    manifest_cipher: Option<Vec<u8>>,
    artifact: Option<Vec<u8>>,
    mapping: Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct Mapping {
    source: Uuid,
    target: Uuid,
    environment: Uuid,
    name: String,
    worker: bool,
    was_running: bool,
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/projects/{id}/backups", get(list).post(create))
        .route("/projects/{id}/backups/{job}/download", get(download))
        .route(
            "/project-backups/import",
            post(import)
                .layer(DefaultBodyLimit::max(format::LIMIT + 128))
                .layer(axum::middleware::from_fn(uploads::heavy_guard)),
        )
        .route("/project-backups/{id}", get(detail))
        .route("/project-backups/{id}/restore", post(restore))
        .route("/project-backups/{id}/cutover", post(cutover))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
pub(crate) async fn lock_app(
    conn: &mut PgConnection,
    app: Uuid,
    organization: Uuid,
) -> Result<(), ApiError> {
    let project:Option<Uuid>=sqlx::query_scalar("SELECT p.id FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2").bind(app).bind(organization).fetch_optional(&mut *conn).await?;
    if let Some(project) = project {
        for attempt in 0..2 {
            let locked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM project_maintenance WHERE project_id=$1)",
            )
            .bind(project)
            .fetch_one(&mut *conn)
            .await?;
            if locked {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "Project maintenance is active. Wait for coordinated backup/restore to finish.",
                ));
            }
            if attempt == 0 {
                sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE")
                    .bind(project)
                    .execute(&mut *conn)
                    .await?;
            }
        }
    }
    Ok(())
}
async fn owned(state: &AppState, id: &Identity, job: Uuid) -> Result<Bundle, ApiError> {
    id.manager()?;
    sqlx::query_as("SELECT * FROM project_bundle_jobs WHERE id=$1 AND organization_id=$2")
        .bind(job)
        .bind(id.organization_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(missing)
}
async fn list(
    State(s): State<AppState>,
    id: Identity,
    Path(project): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    project_access(&s, &id, project).await?;
    let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'operation',operation,'status',status,'message',message,'project_name',project_name,'target_project_id',target_project_id,'sha256',sha256,'created_at',created_at,'finished_at',finished_at,'size_bytes',octet_length(artifact),'mapping',mapping) FROM project_bundle_jobs WHERE organization_id=$1 AND (project_id=$2 OR target_project_id=$2) ORDER BY created_at DESC LIMIT 30").bind(id.organization_id).bind(project).fetch_all(&s.pool).await?;
    Ok(Json(json!(rows)))
}
#[derive(Deserialize)]
struct Consent {
    offline: bool,
}
async fn create(
    State(s): State<AppState>,
    id: Identity,
    Path(project): Path<Uuid>,
    Json(c): Json<Consent>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    if !c.offline {
        return Err(invalid(
            "Confirm project-wide downtime and pause external database writers before continuing.",
        ));
    }
    project_access(&s, &id, project).await?;
    let mut tx = s.pool.begin().await?;
    let label: String = sqlx::query_scalar("SELECT name FROM projects WHERE id=$1 FOR UPDATE")
        .bind(project)
        .fetch_one(&mut *tx)
        .await?;
    let managed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_databases m JOIN applications a ON a.id=m.service_id JOIN environments e ON e.id=a.environment_id WHERE e.project_id=$1)")
        .bind(project).fetch_one(&mut *tx).await?;
    if managed {
        return Err(invalid(
            "Project bundles do not yet recover managed database mappings. Back up each logical database explicitly; a configured-database backup does not include sibling databases.",
        ));
    }
    let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_maintenance WHERE project_id=$1) OR EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=$1 AND (a.restore_blocked OR a.cutover_pending OR EXISTS(SELECT 1 FROM terminal_connections t WHERE t.service_id=a.id AND t.closed_at IS NULL AND t.expires_at>now()) OR EXISTS(SELECT 1 FROM deployments d WHERE d.application_id=a.id AND (d.status IN ('queued','running') OR d.restore_container_id IS NOT NULL)) OR EXISTS(SELECT 1 FROM database_jobs j WHERE (j.application_id=a.id OR j.source_application_id=a.id) AND j.status IN ('queued','running'))))").bind(project).fetch_one(&mut *tx).await?;
    if busy {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Close project terminals and wait for operations/restore recovery to finish first.",
        ));
    }
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        String,
        String,
        String,
        i32,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows:Vec<Row>=sqlx::query_as("SELECT a.id,COALESCE(a.data_origin_id,a.id),a.name,e.name,a.service_kind,d.image,d.image_id,d.container_port,d.service_cipher,d.env_cipher,d.source_repository,d.git_commit,d.source_archive_sha256 FROM applications a JOIN environments e ON e.id=a.environment_id LEFT JOIN deployments d ON d.id=a.active_deployment_id WHERE e.project_id=$1 ORDER BY a.id").bind(project).fetch_all(&mut *tx).await.map_err(|_|invalid("Deploy every service successfully before creating a project bundle."))?;
    if rows.is_empty() || rows.len() > 20 {
        return Err(invalid("Project bundles support 1–20 deployed services."));
    }
    let mut services = Vec::new();
    for (
        app,
        lineage,
        label,
        env,
        kind,
        image,
        image_id,
        port,
        cipher,
        values,
        repo,
        commit,
        upload,
    ) in rows
    {
        let runtime = if kind == "application" {
            serde_json::to_value(application_runtime::decode(&s, app, cipher.as_deref())?)
                .map_err(|_| internal())?
        } else {
            let config: services::DatabaseConfig =
                services::open(&s, app, cipher.as_deref().ok_or_else(internal)?)?;
            if !matches!(
                config.engine,
                services::DatabaseEngine::Postgres | services::DatabaseEngine::Mariadb
            ) {
                return Err(invalid(
                    "Project bundles support PostgreSQL/MariaDB and Applications only.",
                ));
            }
            serde_json::to_value(config).map_err(|_| internal())?
        };
        let worker = runtime["worker_mode"].as_bool().unwrap_or(false);
        let domains:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('hostname',hostname,'route_kind',route_kind,'canonical',canonical) FROM application_domains WHERE application_id=$1 ORDER BY created_at,id").bind(app).fetch_all(&mut *tx).await?;
        services.push(Service {
            id: app,
            lineage,
            name: label,
            environment: env,
            kind,
            image,
            image_id,
            container_port: port,
            runtime,
            environment_values: decrypt(&s, app, values.as_deref())?,
            worker,
            was_running: false,
            backup_sha256: None,
            source_reference: json!({"repository":repo,"commit":commit,"archive_sha256":upload}),
            domains,
        });
    }
    let job = Uuid::new_v4();
    let mut key = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    let key = hex::encode(key);
    let manifest = Manifest {
        version: 1,
        project_id: project,
        name: label.clone(),
        created_at: Utc::now(),
        services,
    };
    sqlx::query("INSERT INTO project_bundle_jobs(id,organization_id,project_id,requested_by,operation,project_name,key_cipher,manifest_cipher) VALUES($1,$2,$3,$4,'backup',$5,$6,$7)").bind(job).bind(id.organization_id).bind(project).bind(id.user_id).bind(label).bind(services::seal(&s,job,&key)?).bind(services::seal(&s,job,&manifest)?).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO project_maintenance(project_id,job_id) VALUES($1,$2)")
        .bind(project)
        .bind(job)
        .execute(&mut *tx)
        .await?;
    for service in &manifest.services {
        sqlx::query("INSERT INTO project_bundle_members(job_id,source_service_id) VALUES($1,$2)")
            .bind(job)
            .bind(service.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"id":job,"recovery_key":key,"message":"Save the recovery key separately from the encrypted bundle. It is shown once."}),
    ))
}
async fn download(
    State(s): State<AppState>,
    id: Identity,
    Path((project, job)): Path<(Uuid, Uuid)>,
) -> Result<axum::response::Response, ApiError> {
    project_access(&s, &id, project).await?;
    let row = owned(&s, &id, job).await?;
    if row.project_id != Some(project) || row.operation != "backup" || row.status != "succeeded" {
        return Err(missing());
    }
    let bytes = row.artifact.ok_or_else(missing)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=ferrix-project.enc",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response())
}
#[derive(Deserialize)]
struct Anchor {
    project: Option<Uuid>,
}
async fn import(
    State(s): State<AppState>,
    id: Identity,
    headers: HeaderMap,
    Query(anchor): Query<Anchor>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    if let Some(project) = anchor.project {
        project_access(&s, &id, project).await?;
    }
    crate::auth::accounts::throttle(&s.pool, &format!("project-import:{}", id.user_id), 5).await?;
    let key = headers
        .get("x-ferrix-bundle-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| invalid("Provide the bundle recovery key."))?
        .to_owned();
    let raw_key = format::key(&key)?;
    let bytes = body.to_vec();
    let (manifest, _) = tokio::task::spawn_blocking(move || format::unpack(&raw_key, &bytes))
        .await
        .map_err(|_| internal())?
        .map_err(|_| invalid("Bundle key, version, checksums or archive validation failed."))?;
    let mut missing_images = Vec::new();
    for service in &manifest.services {
        name(&service.name)?;
        name(&service.environment)?;
        if service.kind == "application" {
            let config: application_runtime::RuntimeConfig =
                serde_json::from_value(service.runtime.clone())
                    .map_err(|_| invalid("Invalid Application runtime snapshot."))?;
            config.validate()?;
        } else {
            let config: services::DatabaseConfig = serde_json::from_value(service.runtime.clone())
                .map_err(|_| invalid("Invalid database snapshot."))?;
            config.validate()?;
            if !matches!(
                config.engine,
                services::DatabaseEngine::Postgres | services::DatabaseEngine::Mariadb
            ) {
                return Err(invalid("Unsupported database engine."));
            }
        }
        if !s
            .docker
            .inspect_image(&service.image_id)
            .await
            .is_ok_and(|i| i.id.as_deref() == Some(service.image_id.as_str()))
        {
            missing_images.push(json!({"service":service.name,"image_id":service.image_id}));
        }
    }
    let job = Uuid::new_v4();
    let mut environments = std::collections::HashMap::new();
    let mapping: Vec<Mapping> = manifest
        .services
        .iter()
        .map(|v| Mapping {
            source: v.id,
            target: Uuid::new_v4(),
            environment: *environments
                .entry(v.environment.clone())
                .or_insert_with(Uuid::new_v4),
            name: v.name.clone(),
            worker: v.worker,
            was_running: v.was_running,
        })
        .collect();
    sqlx::query("INSERT INTO project_bundle_jobs(id,organization_id,requested_by,operation,status,project_name,key_cipher,manifest_cipher,artifact,mapping,message,project_id) VALUES($1,$2,$3,'restore','preview',$4,$5,$6,$7,$8,'Preview ready. No project or containers created.',$9)").bind(job).bind(id.organization_id).bind(id.user_id).bind(&manifest.name).bind(services::seal(&s,job,&key)?).bind(services::seal(&s,job,&manifest)?).bind(body.as_ref()).bind(serde_json::to_value(&mapping).map_err(|_|internal())?).bind(anchor.project).execute(&s.pool).await?;
    sqlx::query("DELETE FROM project_bundle_jobs WHERE organization_id=$1 AND project_id IS NOT DISTINCT FROM $2 AND status='preview' AND id NOT IN(SELECT id FROM project_bundle_jobs WHERE organization_id=$1 AND project_id IS NOT DISTINCT FROM $2 AND status='preview' ORDER BY created_at DESC,id DESC LIMIT 5)").bind(id.organization_id).bind(anchor.project).execute(&s.pool).await?;
    Ok(Json(
        json!({"id":job,"project_name":manifest.name,"mapping":mapping,"missing_images":missing_images,"message":"Restore creates a new isolated project. Domains/automation are not copied. Web services and workers remain stopped; public traffic stays disabled until explicit cutover."}),
    ))
}
#[derive(Deserialize)]
struct Restore {
    name: String,
}
async fn restore(
    State(s): State<AppState>,
    id: Identity,
    Path(job): Path<Uuid>,
    Json(input): Json<Restore>,
) -> Result<StatusCode, ApiError> {
    let row = owned(&s, &id, job).await?;
    if row.status != "preview" {
        return Err(invalid("Import a bundle and review the preview first."));
    }
    let label = name(&input.name)?;
    sqlx::query("UPDATE project_bundle_jobs SET status='queued',project_name=$2,requested_by=$3,message='Restore queued.' WHERE id=$1 AND status='preview'").bind(job).bind(label).bind(id.user_id).execute(&s.pool).await?;
    Ok(StatusCode::ACCEPTED)
}
#[derive(Deserialize)]
struct Cutover {
    source_writers_stopped: bool,
}
async fn cutover(
    State(s): State<AppState>,
    id: Identity,
    Path(job): Path<Uuid>,
    Json(input): Json<Cutover>,
) -> Result<StatusCode, ApiError> {
    let row = owned(&s, &id, job).await?;
    if row.operation != "restore" || row.status != "restored" || !input.source_writers_stopped {
        return Err(invalid(
            "Validate the restored project and confirm source writers are stopped before cutover.",
        ));
    }
    let manifest: Manifest = services::open(
        &s,
        job,
        row.manifest_cipher.as_deref().ok_or_else(internal)?,
    )?;
    let target = row.target_project_id.ok_or_else(missing)?;
    let mut tx = s.pool.begin().await?;
    let projects:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM projects WHERE organization_id=$1 AND (id=$2 OR id=$3) ORDER BY id FOR UPDATE").bind(id.organization_id).bind(target).bind(manifest.project_id).fetch_all(&mut *tx).await?;
    let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_maintenance WHERE project_id=ANY($1)) OR EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=ANY($1) AND (EXISTS(SELECT 1 FROM deployments d WHERE d.application_id=a.id AND d.status IN ('queued','running')) OR EXISTS(SELECT 1 FROM database_jobs j WHERE (j.application_id=a.id OR j.source_application_id=a.id) AND j.status IN ('queued','running')) OR EXISTS(SELECT 1 FROM terminal_connections t WHERE t.service_id=a.id AND t.closed_at IS NULL AND t.expires_at>now())))").bind(&projects).fetch_one(&mut *tx).await?;
    if busy {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Close project terminals and wait for pending operations before cutover.",
        ));
    }
    for project in projects {
        sqlx::query("INSERT INTO project_maintenance(project_id,job_id) VALUES($1,$2)")
            .bind(project)
            .bind(job)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE project_bundle_jobs SET status='cutover_queued',requested_by=$2,message='Cutover queued.' WHERE id=$1 AND status='restored'").bind(job).bind(id.user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::ACCEPTED)
}
pub(super) async fn cycle(
    state: &AppState,
    conn: &mut PgConnection,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool, String> {
    let row:Option<Bundle>=sqlx::query_as("SELECT * FROM project_bundle_jobs WHERE status IN ('queued','running','resuming','cutover_queued','cutting_over') ORDER BY created_at,id LIMIT 1").fetch_optional(&mut *conn).await.map_err(|_|"Could not read coordinated project jobs.")?;
    let Some(row) = row else { return Ok(false) };
    sqlx::query("SELECT set_config('ferrix.bundle_context',$1,false)")
        .bind(row.id.to_string())
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not enter project recovery context.")?;
    let result = operations::run(state, conn, &row, shutdown).await;
    let reset = sqlx::query("SELECT set_config('ferrix.bundle_context','',false)")
        .execute(&mut *conn)
        .await;
    if reset.is_err() {
        return Err("Could not clear project recovery context.".into());
    }
    result?;
    Ok(true)
}

async fn detail(
    State(s): State<AppState>,
    id: Identity,
    Path(job): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    owned(&s, &id, job).await?;
    let row:Value=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'status',status,'operation',operation,'message',message,'target_project_id',target_project_id,'mapping',mapping,'project_name',project_name) FROM project_bundle_jobs WHERE id=$1").bind(job).fetch_one(&s.pool).await?;
    Ok(Json(row))
}

#[cfg(test)]
mod tests;

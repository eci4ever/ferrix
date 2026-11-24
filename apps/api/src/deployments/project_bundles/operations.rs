use super::*;
use sha2::{Digest, Sha256};
use std::time::Duration;
fn failure<T>(_: T) -> String {
    "Coordinated project operation could not be completed.".into()
}
async fn status(
    conn: &mut PgConnection,
    job: Uuid,
    value: &str,
    message: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE project_bundle_jobs SET status=$2,message=$3 WHERE id=$1")
        .bind(job)
        .bind(value)
        .bind(message)
        .execute(conn)
        .await
        .map_err(failure)?;
    Ok(())
}
async fn pause(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Bundle,
    service: Uuid,
) -> Result<(), String> {
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT CASE WHEN resume_needed THEN container_id END FROM project_bundle_members WHERE job_id=$1 AND source_service_id=$2",
    )
    .bind(job.id)
    .bind(service)
    .fetch_one(&mut *conn)
    .await
    .map_err(failure)?;
    let container = if let Some(c) = previous {
        c
    } else {
        let container: String =
            sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1")
                .bind(service)
                .fetch_one(&mut *conn)
                .await
                .map_err(|_| "A source Application is not deployed.")?;
        let info = worker::inspect_owned(state, service, &container)
            .await
            .map_err(failure)?;
        let policy = info
            .host_config
            .and_then(|h| h.restart_policy)
            .unwrap_or_default();
        let running = info.state.and_then(|s| s.running) == Some(true);
        sqlx::query("UPDATE project_bundle_members SET container_id=$3,restart_policy=$4,was_running=$5,resume_needed=true,stop_confirmed=false WHERE job_id=$1 AND source_service_id=$2").bind(job.id).bind(service).bind(&container).bind(serde_json::to_value(policy).map_err(failure)?).bind(running).execute(&mut *conn).await.map_err(failure)?;
        container
    };
    worker::inspect_owned(state, service, &container)
        .await
        .map_err(failure)?;
    state
        .docker
        .update_container(
            &container,
            ContainerUpdateBody {
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::NO),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            },
        )
        .await
        .map_err(failure)?;
    stop(state, &container).await?;
    sqlx::query("UPDATE project_bundle_members SET stop_confirmed=true WHERE job_id=$1 AND source_service_id=$2").bind(job.id).bind(service).execute(&mut *conn).await.map_err(failure)?;
    Ok(())
}
async fn resume(state: &AppState, conn: &mut PgConnection, job: Uuid) -> Result<(), String> {
    type Member = (Uuid, String, Value, bool, bool);
    let rows:Vec<Member>=sqlx::query_as("SELECT source_service_id,container_id,restart_policy,was_running,stop_confirmed FROM project_bundle_members WHERE job_id=$1 AND resume_needed ORDER BY source_service_id").bind(job).fetch_all(&mut *conn).await.map_err(failure)?;
    for (service, container, policy, was_running, confirmed) in rows {
        worker::inspect_owned(state, service, &container)
            .await
            .map_err(failure)?;
        if !confirmed {
            stop(state, &container).await?;
        }
        let mut guard = application_runtime::lineage_guard(state, service).await?;
        if was_running {
            let cipher:Option<Vec<u8>>=sqlx::query_scalar("SELECT d.service_cipher FROM applications a JOIN deployments d ON d.id=a.active_deployment_id WHERE a.id=$1").bind(service).fetch_one(&mut *conn).await.map_err(failure)?;
            let runtime =
                application_runtime::decode(state, service, cipher.as_deref()).map_err(failure)?;
            application_runtime::ensure_single_writer(state, service, &runtime, &mut guard)
                .await
                .map_err(failure)?;
        }
        state
            .docker
            .update_container(
                &container,
                ContainerUpdateBody {
                    restart_policy: Some(serde_json::from_value(policy).map_err(failure)?),
                    ..Default::default()
                },
            )
            .await
            .map_err(failure)?;
        if was_running {
            start(state, &container).await?;
        }
        sqlx::query("UPDATE project_bundle_members SET resume_needed=false WHERE job_id=$1 AND source_service_id=$2").bind(job).bind(service).execute(&mut *conn).await.map_err(failure)?;
    }
    Ok(())
}
async fn release(conn: &mut PgConnection, job: Uuid) -> Result<(), String> {
    sqlx::query("DELETE FROM project_maintenance WHERE job_id=$1")
        .bind(job)
        .execute(conn)
        .await
        .map_err(failure)?;
    Ok(())
}
async fn backup(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Bundle,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let mut manifest: Manifest = services::open(
        state,
        job.id,
        job.manifest_cipher
            .as_deref()
            .ok_or("Missing project snapshot.")?,
    )
    .map_err(failure)?;
    // Pause every managed writer before taking the first related database/file snapshot.
    for service in &manifest.services {
        if service.kind == "application" {
            pause(state, conn, job, service.id).await?;
        }
    }
    let mut files = Vec::new();
    let mut total = 0;
    for service in &mut manifest.services {
        if *shutdown.borrow() {
            return Err("Project backup interrupted by shutdown.".into());
        }
        if service.kind == "application" {
            service.was_running=sqlx::query_scalar("SELECT was_running FROM project_bundle_members WHERE job_id=$1 AND source_service_id=$2").bind(job.id).bind(service.id).fetch_one(&mut *conn).await.map_err(failure)?;
        }
        let persistent = service.kind == "database"
            || service.runtime.get("storage").is_some_and(|v| !v.is_null());
        if !persistent {
            continue;
        }
        let child = Uuid::new_v4();
        let engine = if service.kind == "application" {
            "application"
        } else if service.image.starts_with("mariadb:") {
            "mariadb"
        } else {
            "postgres"
        };
        sqlx::query("INSERT INTO database_jobs(id,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,application_port,project_bundle_id,deployment_id) SELECT $1,a.id,$2,'backup',$3,d.image,d.image_id,d.service_cipher,d.env_cipher,d.container_port,$4,d.id FROM applications a JOIN deployments d ON d.id=a.active_deployment_id WHERE a.id=$5").bind(child).bind(job.requested_by).bind(engine).bind(job.id).bind(service.id).execute(&mut *conn).await.map_err(failure)?;
        sqlx::query("UPDATE project_bundle_members SET backup_id=$3 WHERE job_id=$1 AND source_service_id=$2").bind(job.id).bind(service.id).bind(child).execute(&mut *conn).await.map_err(failure)?;
        backups::cycle_for(state, conn, shutdown, Some(child)).await?;
        let (saved, size, checksum): (String, Option<i64>, Option<String>) =
            sqlx::query_as("SELECT status,size_bytes,sha256 FROM database_jobs WHERE id=$1")
                .bind(child)
                .fetch_one(&mut *conn)
                .await
                .map_err(failure)?;
        if saved != "succeeded" {
            let message: String =
                sqlx::query_scalar("SELECT message FROM database_jobs WHERE id=$1")
                    .bind(child)
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(failure)?;
            return Err(format!("{} backup failed: {message}", service.name));
        }
        total += size.unwrap_or(0);
        if total > format::LIMIT as i64 - 1024 * 1024 {
            return Err(
                "Project exceeds the 64 MiB portable export limit. Source writers will be resumed."
                    .into(),
            );
        }
        if service.kind == "database" {
            service.was_running = true;
        }
        service.backup_sha256 = checksum;
        files.push((service.id, backups::bundle_read(state, child).await?));
    }
    let key: String = services::open(
        state,
        job.id,
        job.key_cipher.as_deref().ok_or("Missing recovery key.")?,
    )
    .map_err(failure)?;
    let key = format::key(&key).map_err(failure)?;
    let bytes = format::pack(&key, &manifest, &files)?;
    let hash = hex::encode(Sha256::digest(&bytes));
    sqlx::query("UPDATE project_bundle_jobs SET status='resuming',artifact=$2,sha256=$3,message='Encrypted bundle prepared. Resuming source writers.' WHERE id=$1").bind(job.id).bind(bytes).bind(hash).execute(&mut *conn).await.map_err(failure)?;
    Ok(())
}

async fn restore_project(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Bundle,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let key: String = services::open(
        state,
        job.id,
        job.key_cipher.as_deref().ok_or("Missing recovery key.")?,
    )
    .map_err(failure)?;
    let key = format::key(&key).map_err(failure)?;
    let (manifest, files) = format::unpack(
        &key,
        job.artifact.as_deref().ok_or("Missing encrypted bundle.")?,
    )?;
    let mapping: Vec<Mapping> = serde_json::from_value(job.mapping.clone()).map_err(failure)?;
    for service in &manifest.services {
        state.docker.inspect_image(&service.image_id).await.map_err(|_|"An immutable image is missing. Load the original image before restoring this bundle.")?;
    }
    let project = Uuid::new_v4();
    let mut tx = conn.begin().await.map_err(failure)?;
    let reserved = quotas::reserve(state, &mut tx).await.map_err(failure)?;
    sqlx::query("INSERT INTO projects(id,organization_id,name,description) VALUES($1,$2,$3,'Restored from an encrypted project bundle. Public traffic is disabled until cutover.')").bind(project).bind(job.organization_id).bind(&job.project_name).execute(&mut *tx).await.map_err(failure)?;
    let mut envs = HashSet::new();
    for (service, map) in manifest.services.iter().zip(&mapping) {
        if service.id != map.source {
            return Err("Restore mapping does not match the manifest.".into());
        }
        if envs.insert(map.environment) {
            sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,$3)")
                .bind(map.environment)
                .bind(project)
                .bind(&service.environment)
                .execute(&mut *tx)
                .await
                .map_err(failure)?;
        }
        let values: Vec<String> = service
            .environment_values
            .iter()
            .map(|v| {
                let mut v = v.clone();
                for map in &mapping {
                    v = v.replace(
                        &format!("ferrix-{}", map.source.simple()),
                        &format!("ferrix-{}", map.target.simple()),
                    );
                }
                v
            })
            .collect();
        sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,service_cipher,env_cipher,worker_mode,restore_blocked,data_origin_id,traffic_enabled,cutover_pending) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,true,$10,false,$9)").bind(map.target).bind(map.environment).bind(&service.name).bind(&service.image).bind(service.container_port).bind(&service.kind).bind(services::seal(state,map.target,&service.runtime).map_err(failure)?).bind(encrypt(state,map.target,&values).map_err(failure)?).bind(service.worker).bind(service.lineage).execute(&mut *tx).await.map_err(failure)?;
        sqlx::query("INSERT INTO project_bundle_members(job_id,source_service_id,target_service_id,was_running) VALUES($1,$2,$3,$4)").bind(job.id).bind(map.source).bind(map.target).bind(map.was_running).execute(&mut *tx).await.map_err(failure)?;
    }
    quotas::check(state, &mut tx, reserved)
        .await
        .map_err(failure)?;
    sqlx::query("UPDATE project_bundle_jobs SET target_project_id=$2 WHERE id=$1")
        .bind(job.id)
        .bind(project)
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
    sqlx::query("INSERT INTO project_maintenance(project_id,job_id) VALUES($1,$2) ON CONFLICT(project_id) DO NOTHING")
        .bind(project)
        .bind(job.id)
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
    tx.commit().await.map_err(failure)?;
    for (service, map) in manifest.services.iter().zip(&mapping) {
        if *shutdown.borrow() {
            return Err(
                "Project restore interrupted. The isolated target remains blocked for cleanup."
                    .into(),
            );
        }
        let deployment = Uuid::new_v4();
        let mut tx = conn.begin().await.map_err(failure)?;
        let reserved = quotas::reserve(state, &mut tx).await.map_err(failure)?;
        sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,pinned_image_id,container_port,service_kind,service_cipher,env_cipher,worker_mode,start_stopped,status,started_at,project_bundle_id,source_repository,git_commit,source_archive_sha256,requested_by_name) SELECT $1,a.id,$2,a.image,$3,a.container_port,a.service_kind,a.service_cipher,a.env_cipher,a.worker_mode,a.service_kind='application','running',now(),$4,$5,$6,$7,(SELECT name FROM auth_users WHERE id=$2) FROM applications a WHERE a.id=$8").bind(deployment).bind(job.requested_by).bind(&service.image_id).bind(job.id).bind(service.source_reference["repository"].as_str()).bind(service.source_reference["commit"].as_str()).bind(service.source_reference["archive_sha256"].as_str()).bind(map.target).execute(&mut *tx).await.map_err(failure)?;
        quotas::check(state, &mut tx, reserved)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(failure)?;
        let candidate:worker::Job=sqlx::query_as("SELECT d.id,d.application_id,d.image,d.pinned_image_id,d.source_cipher,d.source_upload_id,d.start_stopped,d.container_port,d.env_cipher,d.service_kind,d.service_cipher,p.organization_id,p.id AS project_id,e.id AS environment_id FROM deployments d JOIN applications a ON a.id=d.application_id JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE d.id=$1").bind(deployment).fetch_one(&mut *conn).await.map_err(failure)?;
        worker::execute(state, conn, &candidate).await?;
        if let Some((_, bytes)) = files.iter().find(|(id, _)| *id == service.id) {
            let backup = Uuid::new_v4();
            let engine = if service.kind == "application" {
                "application"
            } else if service.image.starts_with("mariadb:") {
                "mariadb"
            } else {
                "postgres"
            };
            sqlx::query("INSERT INTO database_jobs(id,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,application_port,status,size_bytes,sha256,finished_at,project_bundle_id,message) SELECT $1,a.id,$2,'backup',$3,a.image,$4,a.service_cipher,a.env_cipher,a.container_port,'succeeded',$5,$6,now(),$7,'Imported project snapshot.' FROM applications a WHERE a.id=$8").bind(backup).bind(job.requested_by).bind(engine).bind(&service.image_id).bind(bytes.len() as i64).bind(&service.backup_sha256).bind(job.id).bind(map.target).execute(&mut *conn).await.map_err(failure)?;
            backups::bundle_write(state, backup, bytes.clone()).await?;
            let restore = Uuid::new_v4();
            sqlx::query("INSERT INTO database_jobs(id,application_id,source_application_id,source_backup_id,deployment_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,application_port,project_bundle_id) SELECT $1,a.id,a.id,$2,$3,$4,'restore',$5,a.image,$6,a.service_cipher,a.env_cipher,a.container_port,$7 FROM applications a WHERE a.id=$8").bind(restore).bind(backup).bind(deployment).bind(job.requested_by).bind(engine).bind(&service.image_id).bind(job.id).bind(map.target).execute(&mut *conn).await.map_err(failure)?;
            backups::cycle_for(state, conn, shutdown, Some(restore)).await?;
            let complete: bool =
                sqlx::query_scalar("SELECT status='succeeded' FROM database_jobs WHERE id=$1")
                    .bind(restore)
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(failure)?;
            if !complete {
                return Err("A project member restore failed. The new project remains isolated; the source is unchanged.".into());
            }
        }
    }
    sqlx::query("UPDATE applications a SET restore_blocked=false FROM environments e WHERE a.environment_id=e.id AND e.project_id=$1").bind(project).execute(&mut *conn).await.map_err(failure)?;
    status(conn,job.id,"restored","Restored to a new isolated project. Validate via server-only loopback ports. Workers and public traffic remain disabled until cutover.").await?;
    sqlx::query("UPDATE project_bundle_jobs SET key_cipher=NULL,artifact=NULL,finished_at=now() WHERE id=$1")
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(failure)?;
    retire_snapshots(conn, job.id).await?;
    release(conn, job.id).await
}
async fn stop_targets(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Bundle,
) -> Result<(), String> {
    let project: Option<Uuid> =
        sqlx::query_scalar("SELECT target_project_id FROM project_bundle_jobs WHERE id=$1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await
            .map_err(failure)?;
    if let Some(project) = project {
        let rows:Vec<(Uuid,String)>=sqlx::query_as("SELECT a.id,a.active_container_id FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=$1 AND a.service_kind='application' AND a.active_container_id IS NOT NULL").bind(project).fetch_all(&mut *conn).await.map_err(failure)?;
        sqlx::query("UPDATE applications a SET traffic_enabled=false FROM environments e WHERE a.environment_id=e.id AND e.project_id=$1").bind(project).execute(&mut *conn).await.map_err(failure)?;
        for (app, container) in rows {
            worker::inspect_owned(state, app, &container)
                .await
                .map_err(failure)?;
            state
                .docker
                .update_container(
                    &container,
                    ContainerUpdateBody {
                        restart_policy: Some(RestartPolicy {
                            name: Some(RestartPolicyNameEnum::NO),
                            maximum_retry_count: None,
                        }),
                        ..Default::default()
                    },
                )
                .await
                .map_err(failure)?;
            stop(state, &container).await?;
        }
    }
    Ok(())
}
async fn start_target(
    state: &AppState,
    conn: &mut PgConnection,
    service: Uuid,
    job: Uuid,
) -> Result<(), String> {
    let (container,cipher,kind):(String,Option<Vec<u8>>,String)=sqlx::query_as("SELECT a.active_container_id,d.service_cipher,a.service_kind FROM applications a JOIN deployments d ON d.id=a.active_deployment_id WHERE a.id=$1 AND NOT a.restore_blocked").bind(service).fetch_one(&mut *conn).await.map_err(failure)?;
    if kind == "application" {
        let runtime =
            application_runtime::decode(state, service, cipher.as_deref()).map_err(failure)?;
        let mut guard = application_runtime::lineage_guard(state, service).await?;
        sqlx::query("SELECT set_config('ferrix.bundle_context',$1,true)")
            .bind(job.to_string())
            .execute(&mut *guard)
            .await
            .map_err(failure)?;
        application_runtime::ensure_single_writer(state, service, &runtime, &mut guard)
            .await
            .map_err(failure)?;
    }
    worker::inspect_owned(state, service, &container)
        .await
        .map_err(failure)?;
    state
        .docker
        .update_container(
            &container,
            ContainerUpdateBody {
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            },
        )
        .await
        .map_err(failure)?;
    start(state, &container).await?;
    let mut stable = 0;
    for _ in 0..30 {
        let info = worker::inspect_owned(state, service, &container)
            .await
            .map_err(failure)?;
        let state = info.state.ok_or("Restored runtime state unavailable.")?;
        if state.running != Some(true) {
            return Err(
                "Restored instance exited during cutover. Source recovery will run.".into(),
            );
        }
        let healthy = state.health.and_then(|h| h.status).map(|h| h.to_string());
        if healthy.as_deref() == Some("unhealthy") {
            return Err("Restored instance is unhealthy. Source recovery will run.".into());
        }
        if healthy.is_none() || healthy.as_deref() == Some("healthy") {
            stable += 1;
            if stable >= 2 {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("Restored instance did not become ready. Source recovery will run.".into())
}
async fn cutover(state: &AppState, conn: &mut PgConnection, job: &Bundle) -> Result<(), String> {
    let manifest: Manifest = services::open(
        state,
        job.id,
        job.manifest_cipher
            .as_deref()
            .ok_or("Missing cutover manifest.")?,
    )
    .map_err(failure)?;
    let target = job.target_project_id.ok_or("Missing restored project.")?;
    let mut tx = conn.begin().await.map_err(failure)?;
    let source: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM projects WHERE id=$1 AND organization_id=$2 AND id<>$3 FOR UPDATE",
    )
    .bind(manifest.project_id)
    .bind(job.organization_id)
    .bind(target)
    .fetch_optional(&mut *tx)
    .await
    .map_err(failure)?;
    sqlx::query("INSERT INTO project_maintenance(project_id,job_id) VALUES($1,$2) ON CONFLICT(project_id) DO NOTHING")
        .bind(target)
        .bind(job.id)
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
    if let Some(source) = source {
        sqlx::query("INSERT INTO project_maintenance(project_id,job_id) VALUES($1,$2) ON CONFLICT(project_id) DO NOTHING")
            .bind(source)
            .bind(job.id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
    }
    tx.commit().await.map_err(failure)?;
    if let Some(source) = source {
        let writers:Vec<Uuid>=sqlx::query_scalar("SELECT a.id FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=$1 AND a.service_kind='application'").bind(source).fetch_all(&mut *conn).await.map_err(failure)?;
        for service in writers {
            sqlx::query("INSERT INTO project_bundle_members(job_id,source_service_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(job.id).bind(service).execute(&mut *conn).await.map_err(failure)?;
            pause(state, conn, job, service).await?;
        }
    }
    let maps: Vec<Mapping> = serde_json::from_value(job.mapping.clone()).map_err(failure)?;
    for map in &maps {
        if map.was_running {
            start_target(state, conn, map.target, job.id).await?;
        }
    }
    let mut finalize = conn.begin().await.map_err(failure)?;
    if source.is_some() {
        for (service, map) in manifest.services.iter().zip(&maps) {
            let hosts: Vec<String> = service
                .domains
                .iter()
                .filter_map(|d| d["hostname"].as_str().map(str::to_owned))
                .collect();
            if !hosts.is_empty() {
                sqlx::query("UPDATE application_domains SET application_id=$1 WHERE application_id=$2 AND hostname=ANY($3) AND EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$2 AND e.project_id=$4 AND p.organization_id=$5)").bind(map.target).bind(service.id).bind(&hosts).bind(manifest.project_id).bind(job.organization_id).execute(&mut *finalize).await.map_err(failure)?;
            }
        }
    }
    sqlx::query("UPDATE applications a SET traffic_enabled=true,cutover_pending=false FROM environments e WHERE a.environment_id=e.id AND e.project_id=$1").bind(target).execute(&mut *finalize).await.map_err(failure)?;
    sqlx::query("UPDATE project_bundle_members SET resume_needed=false WHERE job_id=$1")
        .bind(job.id)
        .execute(&mut *finalize)
        .await
        .map_err(failure)?;
    status(&mut finalize,job.id,"active","Cutover completed. Target traffic is enabled; original managed Application writers remain stopped.").await?;
    release(&mut finalize, job.id).await?;
    finalize.commit().await.map_err(failure)
}
async fn stop(state: &AppState, container: &str) -> Result<(), String> {
    match state
        .docker
        .stop_container(
            container,
            Some(StopContainerOptionsBuilder::default().t(30).build()),
        )
        .await
    {
        Ok(())
        | Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 304, ..
        }) => Ok(()),
        Err(_) => Err("Could not confirm writer shutdown.".into()),
    }
}
pub(super) async fn run(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Bundle,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users u JOIN auth_members m ON m.user_id=u.id WHERE u.id=$1 AND m.organization_id=$2 AND m.role IN ('owner','admin') AND NOT auth_user_banned(u.banned,u.ban_expires_at))").bind(job.requested_by).bind(job.organization_id).fetch_one(&mut *conn).await.map_err(failure)?;
    if job.operation == "backup" {
        if job.status == "queued" && allowed {
            status(
                conn,
                job.id,
                "running",
                "Pausing all Application writers for a coordinated snapshot.",
            )
            .await?;
            if let Err(message) = backup(state, conn, job, shutdown).await {
                status(conn, job.id, "resuming", &message).await?;
            }
        } else if job.status != "resuming" {
            backups::cycle_for(state, conn, shutdown, Some(Uuid::nil())).await?;
            status(
                conn,
                job.id,
                "resuming",
                "Backup interrupted or authorization changed. Recovering source writers.",
            )
            .await?;
        }
        resume(state, conn, job.id).await?;
        let ready: bool =
            sqlx::query_scalar("SELECT artifact IS NOT NULL FROM project_bundle_jobs WHERE id=$1")
                .bind(job.id)
                .fetch_one(&mut *conn)
                .await
                .map_err(failure)?;
        sqlx::query("UPDATE project_bundle_jobs SET status=$2,key_cipher=NULL,finished_at=now(),message=CASE WHEN $2='succeeded' THEN 'Encrypted bundle ready. All original writer states restored.' ELSE message END WHERE id=$1").bind(job.id).bind(if ready{"succeeded"}else{"failed"}).execute(&mut *conn).await.map_err(failure)?;
        retire_snapshots(conn, job.id).await?;
        release(conn, job.id).await?;
        sqlx::query("UPDATE project_bundle_jobs SET artifact=NULL WHERE project_id=$1 AND operation='backup' AND status IN ('succeeded','failed') AND id NOT IN(SELECT id FROM project_bundle_jobs WHERE project_id=$1 AND operation='backup' AND artifact IS NOT NULL ORDER BY created_at DESC,id DESC LIMIT 5)").bind(job.project_id).execute(&mut *conn).await.map_err(failure)?;
        return Ok(());
    }
    if job.status == "queued" && allowed {
        status(
            conn,
            job.id,
            "running",
            "Preparing an isolated restored project.",
        )
        .await?;
        if let Err(message) = restore_project(state, conn, job, shutdown).await {
            stop_targets(state, conn, job).await?;
            status(conn, job.id, "failed", &message).await?;
            discard_failed_import(conn, job.id).await?;
            release(conn, job.id).await?;
        }
    } else if job.status == "cutover_queued" && allowed {
        status(
            conn,
            job.id,
            "cutting_over",
            "Stopping original managed writers before enabling target workers and traffic.",
        )
        .await?;
        if let Err(message) = cutover(state, conn, job).await {
            stop_targets(state, conn, job).await?;
            resume(state, conn, job.id).await?;
            status(conn, job.id, "restored", &message).await?;
            release(conn, job.id).await?;
        }
    } else {
        backups::cycle_for(state, conn, shutdown, Some(Uuid::nil())).await?;
        stop_targets(state, conn, job).await?;
        resume(state, conn, job.id).await?;
        status(conn,job.id,if matches!(job.status.as_str(),"cutting_over"|"cutover_queued"){"restored"}else{"failed"},"Operation interrupted or permission changed. Source preserved; target traffic remains disabled. Retry or clean up the isolated target.").await?;
        if !matches!(job.status.as_str(), "cutting_over" | "cutover_queued") {
            discard_failed_import(conn, job.id).await?;
        }
        release(conn, job.id).await?;
    }
    Ok(())
}

async fn start(state: &AppState, container: &str) -> Result<(), String> {
    match state.docker.start_container(container, None).await {
        Ok(())
        | Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 304, ..
        }) => Ok(()),
        Err(_) => Err("Could not start the selected instance.".into()),
    }
}

async fn retire_snapshots(conn: &mut PgConnection, job: Uuid) -> Result<(), String> {
    sqlx::query("UPDATE database_jobs SET pruning=true,prune_retry_at=now() WHERE project_bundle_id=$1 AND operation='backup' AND pruned_at IS NULL").bind(job).execute(conn).await.map_err(failure)?;
    Ok(())
}
async fn discard_failed_import(conn: &mut PgConnection, job: Uuid) -> Result<(), String> {
    retire_snapshots(conn, job).await?;
    sqlx::query("UPDATE project_bundle_jobs SET key_cipher=NULL,artifact=NULL,manifest_cipher=NULL,finished_at=now() WHERE id=$1 AND status='failed'").bind(job).execute(conn).await.map_err(failure)?;
    Ok(())
}

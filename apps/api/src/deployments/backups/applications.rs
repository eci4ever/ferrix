use super::*;
use crate::deployments::application_runtime::{self, RuntimeConfig};
use bollard::{
    models::{RestartPolicy, RestartPolicyNameEnum},
    query_parameters::{CreateImageOptionsBuilder, StopContainerOptionsBuilder},
};

const IMAGE: &str = "python:3.13-slim";
const SCRIPT: &str = include_str!("application_archive.py");
pub(super) async fn helper_image(state: &AppState) -> Result<String, String> {
    if let Ok(info) = state.docker.inspect_image(IMAGE).await {
        return info
            .id
            .ok_or_else(|| "Archive helper has no immutable image ID.".into());
    }
    let mut pull = state.docker.create_image(
        Some(
            CreateImageOptionsBuilder::default()
                .from_image(IMAGE)
                .build(),
        ),
        None,
        None,
    );
    while let Some(item) = pull.next().await {
        let item = item.map_err(|_| "Could not pull the official archive helper image.")?;
        if item.error_detail.is_some() {
            return Err("Archive helper image is unavailable.".into());
        }
    }
    state
        .docker
        .inspect_image(IMAGE)
        .await
        .map_err(|_| "Could not inspect archive helper image.")?
        .id
        .ok_or_else(|| "Archive helper image has no ID.".into())
}
async fn data_volume(state: &AppState, service: Uuid) -> Result<String, String> {
    let (org,project):(Uuid,Uuid)=sqlx::query_as("SELECT p.organization_id,p.id FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1").bind(service).fetch_one(&state.pool).await.map_err(|_|"Could not verify Application storage ownership.")?;
    let name = application_runtime::volume(service);
    let info = state
        .docker
        .inspect_volume(&name)
        .await
        .map_err(|_| "Application storage is unavailable.")?;
    if info.driver != "local"
        || !info.options.is_empty()
        || info.labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
        || info.labels.get("io.ferrix.application") != Some(&service.to_string())
        || info.labels.get("io.ferrix.organization") != Some(&org.to_string())
        || info.labels.get("io.ferrix.project") != Some(&project.to_string())
    {
        return Err("Application volume ownership or driver could not be verified.".into());
    }
    Ok(name)
}
async fn pause(state: &AppState, conn: &mut PgConnection, job: &Job) -> Result<(), String> {
    let _lineage = application_runtime::lineage_guard(state, job.application_id).await?;
    let (container, active): (Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_container_id,active_deployment_id FROM applications WHERE id=$1",
    )
    .bind(job.application_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| "Could not read Application runtime.")?;
    let container = container.ok_or("Application has no deployed container.")?;
    let expected: Option<Uuid> =
        sqlx::query_scalar("SELECT deployment_id FROM database_jobs WHERE id=$1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|_| "Could not verify backup snapshot.")?;
    if active != expected {
        return Err("Application version changed before backup started.".into());
    }
    let info = worker::inspect_owned(state, job.application_id, &container)
        .await
        .map_err(|_| "Could not verify the Application container.")?;
    let was_running = info.state.and_then(|s| s.running) == Some(true);
    let restart =
        serde_json::to_string(&info.host_config.and_then(|h| h.restart_policy).unwrap_or(
            RestartPolicy {
                name: Some(RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            },
        ))
        .map_err(|_| "Could not snapshot restart policy.")?;
    sqlx::query("UPDATE database_jobs SET resume_container_id=$2,resume_was_running=$3,resume_restart_policy=$4,resume_needed=true,message='Stopping Application for an offline volume backup.' WHERE id=$1 AND status='running'").bind(job.id).bind(&container).bind(was_running).bind(restart).execute(&mut *conn).await.map_err(|_|"Could not persist Application recovery state.")?;
    state
        .docker
        .update_container(
            &container,
            bollard::models::ContainerUpdateBody {
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::NO),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| "Could not disable automatic Application restart safely.")?;
    match state
        .docker
        .stop_container(
            &container,
            Some(StopContainerOptionsBuilder::default().t(30).build()),
        )
        .await
    {
        Ok(())
        | Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 304, ..
        }) => {}
        _ => return Err("Could not stop Application for offline backup.".into()),
    }
    sqlx::query("UPDATE database_jobs SET resume_stop_confirmed=true WHERE id=$1")
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not confirm offline stop.")?;
    Ok(())
}
pub(super) async fn resume(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Job,
) -> Result<(), String> {
    let pending:Option<(String,bool,Option<String>,bool)>=sqlx::query_as("SELECT resume_container_id,resume_was_running,resume_restart_policy,resume_stop_confirmed FROM database_jobs WHERE id=$1 AND resume_needed").bind(job.id).fetch_optional(&mut *conn).await.map_err(|_|"Could not read Application backup recovery state.")?;
    let Some((container, was_running, policy, stop_confirmed)) = pending else {
        return Ok(());
    };
    let _lineage = application_runtime::lineage_guard(state, job.application_id).await?;
    let active: Option<String> =
        sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1")
            .bind(job.application_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|_| "Could not verify Application recovery target.")?;
    if active.as_deref() != Some(&container) {
        return Err("Application recovery target changed. Keep the service stopped and review its backup job.".into());
    }
    let info = worker::inspect_owned(state, job.application_id, &container)
        .await
        .map_err(|_| "Could not verify Application container for recovery.")?;
    // A cancelled Docker Stop request may still be completing on the daemon.
    // Confirm it has finished before restoring the policy or starting the source.
    if !stop_confirmed {
        match state
            .docker
            .stop_container(
                &container,
                Some(StopContainerOptionsBuilder::default().t(30).build()),
            )
            .await
        {
            Ok(())
            | Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            }) => {}
            _ => {
                return Err(
                    "Could not confirm the interrupted offline stop. Recovery will retry.".into(),
                );
            }
        }
        sqlx::query("UPDATE database_jobs SET resume_stop_confirmed=true WHERE id=$1")
            .bind(job.id)
            .execute(&mut *conn)
            .await
            .map_err(|_| "Could not persist recovered stop confirmation.")?;
    }
    let info = if stop_confirmed {
        info
    } else {
        worker::inspect_owned(state, job.application_id, &container)
            .await
            .map_err(|_| "Could not verify recovered container state.")?
    };
    let running = info.state.and_then(|s| s.running) == Some(true);
    if was_running && !running {
        let runtime: RuntimeConfig = services::open(state, job.application_id, &job.config_cipher)
            .map_err(|_| "Could not decrypt backup recovery settings.")?;
        application_runtime::ensure_single_writer(state, job.application_id, &runtime, conn)
            .await?;
    }
    state
        .docker
        .update_container(
            &container,
            bollard::models::ContainerUpdateBody {
                restart_policy: Some(
                    policy
                        .as_deref()
                        .map(serde_json::from_str::<RestartPolicy>)
                        .transpose()
                        .map_err(|_| "Invalid saved restart policy.")?
                        .unwrap_or(RestartPolicy {
                            name: Some(RestartPolicyNameEnum::NO),
                            maximum_retry_count: None,
                        }),
                ),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| "Could not restore Application restart policy.")?;
    if was_running && !running {
        state
            .docker
            .start_container(&container, None)
            .await
            .map_err(|_| "Could not resume Application after backup.")?;
    }
    sqlx::query("UPDATE database_jobs SET resume_needed=false WHERE id=$1")
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not confirm Application recovery.")?;
    Ok(())
}
pub(super) async fn queue_restore(
    state: &AppState,
    id: &Identity,
    source: &Application,
    saved: &Job,
    label: &str,
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    key: Option<&crate::auth::api_keys::KeyAccess>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let reserved = quotas::reserve(state, &mut tx).await?;
    let target = Uuid::new_v4();
    let deployment = Uuid::new_v4();
    let job = Uuid::new_v4();
    let runtime: RuntimeConfig = services::open(state, source.id, &saved.config_cipher)?;
    if runtime.storage.is_none() {
        return Err(invalid(
            "The backup has no persistent Application storage snapshot.",
        ));
    }
    let cipher = services::seal(state, target, &runtime)?;
    let env = encrypt(
        state,
        target,
        &decrypt(state, source.id, saved.env_cipher.as_deref())?,
    )?;
    let lineage: Uuid =
        sqlx::query_scalar("SELECT COALESCE(data_origin_id,id) FROM applications WHERE id=$1")
            .bind(source.id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,service_cipher,env_cipher,worker_mode,restore_blocked,data_origin_id) VALUES($1,$2,$3,$4,$5,'application',$6,$7,$8,true,$9)").bind(target).bind(source.environment_id).bind(label).bind(&saved.image).bind(saved.application_port).bind(&cipher).bind(&env).bind(runtime.worker_mode).bind(lineage).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port,env_cipher,service_kind,service_cipher,pinned_image_id,requested_by_name,requested_by_key_id,requested_by_key_name,worker_mode,start_stopped) VALUES($1,$2,$3,$4,$5,$6,'application',$7,$8,$9,$10,$11,$12,true)").bind(deployment).bind(target).bind(id.user_id).bind(&saved.image).bind(saved.application_port).bind(&env).bind(&cipher).bind(&saved.image_id).bind(&id.name).bind(key.map(|v|v.id)).bind(key.map(|v|v.name.as_str())).bind(runtime.worker_mode).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO database_jobs(id,application_id,source_application_id,source_backup_id,deployment_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,s3_cipher,s3_key,application_port) VALUES($1,$2,$3,$4,$5,$6,'restore','application',$7,$8,$9,$10,$11,$12,$13)").bind(job).bind(target).bind(source.id).bind(saved.id).bind(deployment).bind(id.user_id).bind(&saved.image).bind(&saved.image_id).bind(cipher).bind(env).bind(&saved.s3_cipher).bind(&saved.s3_key).bind(saved.application_port).execute(&mut *tx).await?;
    quotas::check(state, &mut tx, reserved).await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            json!({"id":job,"status":"queued","application_id":target,"project_id":source.project_id}),
        ),
    ))
}
pub(super) async fn execute(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Job,
) -> Result<(Option<i64>, Option<String>), String> {
    let image = helper_image(state).await?; // Pull before stopping the source.
    let restoring = job.operation == "restore";
    let runtime: RuntimeConfig = services::open(state, job.application_id, &job.config_cipher)
        .map_err(|_| "Could not decrypt Application backup settings.")?;
    if runtime.storage.is_none() {
        return Err("Application storage is not configured.".into());
    }
    let data = data_volume(state, job.application_id).await?;
    if !restoring {
        pause(state, conn, job).await?;
    } else {
        let container: Option<String> =
            sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1")
                .bind(job.application_id)
                .fetch_one(&mut *conn)
                .await
                .map_err(|_| "Could not read restore target.")?;
        let info = worker::inspect_owned(
            state,
            job.application_id,
            &container.ok_or("Restore target was not prepared.")?,
        )
        .await
        .map_err(|_| "Could not verify restore target.")?;
        if info.state.and_then(|s| s.running) == Some(true) {
            return Err("Restore target must remain stopped.".into());
        }
    }
    // Restore writes only its new owned volume. A related source may still run;
    // no target process is started and lineage checks protect any later Start.
    ensure_volume_idle(state, job.application_id).await?;
    let source = job.source_application_id.unwrap_or(job.application_id);
    let file = job.source_backup_id.unwrap_or(job.id);
    if restoring && job.s3_cipher.is_some() {
        remote::download(state, conn, job, source, file).await?;
        cleanup_helper(state, job).await?;
    }
    let backups = storage(state, source, !restoring).await?;
    let checksum: Option<String> = if restoring {
        sqlx::query_scalar("SELECT sha256 FROM database_jobs WHERE id=$1 AND application_id=$2 AND status='succeeded' AND NOT pruning AND pruned_at IS NULL").bind(file).bind(source).fetch_optional(&mut *conn).await.map_err(|_|"Could not read archive integrity metadata.")?.flatten()
    } else {
        None
    };
    if restoring && checksum.is_none() {
        return Err("The original backup is unavailable.".into());
    }
    let mut args = vec![
        SCRIPT.into(),
        if restoring {
            "restore".into()
        } else {
            "backup".into()
        },
        "/data".into(),
        format!("/backups/{}.dump", file.simple()),
    ];
    if let Some(hash) = checksum {
        args.push(hash);
    }
    let helper = helper_name(job.id);
    let config = ContainerCreateBody {
        image: Some(image),
        user: Some("0:0".into()),
        entrypoint: Some(vec!["python3".into(), "-c".into()]),
        cmd: Some(args),
        labels: Some(HashMap::from([
            ("io.ferrix.managed".into(), "true".into()),
            (
                "io.ferrix.application".into(),
                job.application_id.to_string(),
            ),
            ("io.ferrix.backup-job".into(), job.id.to_string()),
        ])),
        host_config: Some(HostConfig {
            network_mode: Some("none".into()),
            readonly_rootfs: Some(true),
            memory: Some(512 * 1024 * 1024),
            nano_cpus: Some(1_000_000_000),
            pids_limit: Some(32),
            cap_drop: Some(vec!["ALL".into()]),
            cap_add: Some(vec![
                "CHOWN".into(),
                "DAC_OVERRIDE".into(),
                "FOWNER".into(),
                "FSETID".into(),
            ]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            mounts: Some(vec![
                Mount {
                    typ: Some(MountTypeEnum::VOLUME),
                    source: Some(data),
                    target: Some("/data".into()),
                    read_only: Some(!restoring),
                    volume_options: Some(bollard::models::MountVolumeOptions {
                        no_copy: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Mount {
                    typ: Some(MountTypeEnum::VOLUME),
                    source: Some(backups),
                    target: Some("/backups".into()),
                    read_only: Some(restoring),
                    ..Default::default()
                },
            ]),
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
    state
        .docker
        .create_container(
            Some(
                CreateContainerOptionsBuilder::default()
                    .name(&helper)
                    .build(),
            ),
            config,
        )
        .await
        .map_err(|_| "Could not create the offline Application archive helper.")?;
    state
        .docker
        .start_container(&helper, None)
        .await
        .map_err(|_| "Could not start the Application archive helper.")?;
    loop {
        let info = state
            .docker
            .inspect_container(&helper, None)
            .await
            .map_err(|_| "Could not inspect Application archive progress.")?;
        let status = info.state.ok_or("Archive helper returned no state.")?;
        if status.running != Some(true) {
            if status.exit_code != Some(0) {
                return Err("Application archive failed. Check disk space, file permissions, supported entries, an empty restore target and archive integrity.".into());
            }
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if restoring {
        return Ok((None, None));
    }
    let (size, hash) = helper_output(state, &helper).await?;
    cleanup_helper(state, job).await?;
    resume(state, conn, job).await?;
    if job.s3_cipher.is_some() {
        remote::upload(state, job, size, &hash).await?;
    }
    Ok((Some(size), Some(hash)))
}

async fn ensure_volume_idle(state: &AppState, service: Uuid) -> Result<(), String> {
    let filters = HashMap::from([(
        "volume".to_owned(),
        vec![application_runtime::volume(service)],
    )]);
    let containers = state
        .docker
        .list_containers(Some(
            bollard::query_parameters::ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filters)
                .build(),
        ))
        .await
        .map_err(|_| "Could not verify Application volume writers.")?;
    for container in containers {
        let info = state
            .docker
            .inspect_container(&container.id.ok_or("Unknown volume container.")?, None)
            .await
            .map_err(|_| "Could not verify Application volume runtime.")?;
        if info.state.and_then(|s| s.running) == Some(true) {
            return Err("Another container is using the Application volume. Offline backup/restore cannot proceed.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

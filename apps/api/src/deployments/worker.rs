use super::*;
use bollard::{
    models::{
        ContainerCreateBody, HostConfig, HostConfigLogConfig, PortBinding, RestartPolicy,
        RestartPolicyNameEnum,
    },
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
        RemoveContainerOptionsBuilder,
    },
};
use futures_util::StreamExt;
use sqlx::PgConnection;
use std::{collections::HashMap, time::Duration};
use tokio::sync::watch;

#[derive(sqlx::FromRow)]
pub(super) struct Job {
    pub(super) id: Uuid,
    pub(super) application_id: Uuid,
    pub(super) image: String,
    pub(super) pinned_image_id: Option<String>,
    pub(super) container_port: i32,
    pub(super) env_cipher: Option<Vec<u8>>,
    pub(super) service_kind: String,
    pub(super) service_cipher: Option<Vec<u8>>,
    pub(super) organization_id: Uuid,
    pub(super) project_id: Uuid,
    pub(super) environment_id: Uuid,
}
fn docker_error() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Docker is unavailable. Check the connected Docker server.",
    )
}
fn labels_match(labels: Option<&HashMap<String, String>>, app: Uuid) -> bool {
    labels.is_some_and(|l| {
        l.get("io.ferrix.managed").map(String::as_str) == Some("true")
            && l.get("io.ferrix.application") == Some(&app.to_string())
    })
}
pub(super) async fn inspect_owned(
    state: &AppState,
    app: Uuid,
    container: &str,
) -> Result<bollard::models::ContainerInspectResponse, ApiError> {
    let info = tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.inspect_container(container, None),
    )
    .await
    .map_err(|_| docker_error())?
    .map_err(|_| docker_error())?;
    if !labels_match(info.config.as_ref().and_then(|c| c.labels.as_ref()), app) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Container ownership could not be verified.",
        ));
    }
    Ok(info)
}
pub(super) async fn runtime(state: &AppState, app: &Application) -> Result<Value, ApiError> {
    let Some(container) = &app.active_container_id else {
        return Ok(json!({"state":"not_deployed","host_port":null}));
    };
    match inspect_owned(state, app.id, container).await {
        Ok(info) => {
            let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
            let host_port = info
                .network_settings
                .as_ref()
                .and_then(|network| network.ports.as_ref())
                .into_iter()
                .flat_map(|ports| ports.values())
                .filter_map(|bindings| bindings.as_ref())
                .flatten()
                .filter_map(|binding| binding.host_port.as_deref())
                .find_map(|port| port.parse::<u16>().ok().filter(|port| *port > 0));
            let health = info
                .state
                .as_ref()
                .and_then(|s| s.health.as_ref())
                .and_then(|h| h.status.as_ref())
                .map(ToString::to_string);
            let status = info
                .state
                .and_then(|s| s.status)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".into());
            Ok(
                json!({"state":status,"health":health,"host_port":if running {host_port}else{None},"container_id":container,"internal_host":format!("ferrix-{}",app.id.simple()),"internal_port":app.container_port}),
            )
        }
        Err(error) if error.0 == StatusCode::CONFLICT => Err(error),
        Err(_) => Ok(json!({"state":"unavailable","host_port":null})),
    }
}
pub(super) async fn phase(conn: &mut PgConnection, job: Uuid, text: &str) -> Result<(), String> {
    let count=sqlx::query("UPDATE deployments SET logs=left(logs || $1 || E'\\n',65536) WHERE id=$2 AND status='running'")
        .bind(text).bind(job).execute(conn).await.map_err(|_|"Could not persist deployment progress.")?.rows_affected();
    if count != 1 {
        return Err("Deployment is no longer running.".into());
    }
    Ok(())
}
async fn execute(state: &AppState, conn: &mut PgConnection, job: &Job) -> Result<(), String> {
    if !matches!(job.service_kind.as_str(), "application" | "database") {
        return Err("This service type is no longer supported.".into());
    }
    let image = if let Some(image_id) = &job.pinned_image_id {
        phase(
            conn,
            job.id,
            "Restoring the selected deployment image and configuration…",
        )
        .await?;
        state.docker.inspect_image(image_id).await.map_err(|_| "The original image is no longer available on this Docker server. Rollback cannot continue." )?
    } else {
        phase(conn, job.id, "Pulling Docker image…").await?;
        let options = CreateImageOptionsBuilder::default()
            .from_image(&job.image)
            .build();
        let mut pull = state.docker.create_image(Some(options), None, None);
        while let Some(event) = pull.next().await {
            let event = event.map_err(
                |_| "Could not pull image. Check its name, tag and registry availability.",
            )?;
            if event.error_detail.is_some() {
                return Err(
                    "The registry could not provide this image. Check its name and tag.".into(),
                );
            }
        }
        state
            .docker
            .inspect_image(&job.image)
            .await
            .map_err(|_| "Could not inspect the pulled image.")?
    };
    if job.service_kind == "application"
        && image
            .config
            .as_ref()
            .and_then(|c| c.volumes.as_ref())
            .is_some_and(|volumes| !volumes.is_empty())
    {
        return Err(
            "This image declares persistent volumes. This MVP supports stateless images only."
                .into(),
        );
    }
    let image_id = image.id.ok_or("Pulled image has no immutable ID.")?;
    sqlx::query("UPDATE deployments SET image_id=$1 WHERE id=$2 AND status='running'")
        .bind(&image_id)
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not persist image ID.")?;
    phase(
        conn,
        job.id,
        "Creating an isolated container with an automatic loopback port…",
    )
    .await?;
    let env = decrypt(state, job.application_id, job.env_cipher.as_deref())
        .map_err(|_| "Could not decrypt environment variables. Check the Ferrix auth key.")?;
    let port_key = format!("{}/tcp", job.container_port);
    let labels = HashMap::from([
        ("io.ferrix.managed".into(), "true".into()),
        (
            "io.ferrix.application".into(),
            job.application_id.to_string(),
        ),
        ("io.ferrix.deployment".into(), job.id.to_string()),
        (
            "io.ferrix.organization".into(),
            job.organization_id.to_string(),
        ),
        ("io.ferrix.project".into(), job.project_id.to_string()),
        (
            "io.ferrix.environment".into(),
            job.environment_id.to_string(),
        ),
    ]);
    let mut config = ContainerCreateBody {
        image: Some(image_id),
        env: Some(env),
        labels: Some(labels),
        exposed_ports: Some(vec![port_key.clone()]),
        host_config: Some(HostConfig {
            port_bindings: Some(HashMap::from([(
                port_key.clone(),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".into()),
                    host_port: Some("0".into()),
                }]),
            )])),
            memory: Some(512 * 1024 * 1024),
            nano_cpus: Some(1_000_000_000),
            pids_limit: Some(256),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".into()),
                config: Some(HashMap::from([
                    ("max-size".into(), "10m".into()),
                    ("max-file".into(), "3".into()),
                ])),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    super::services::database::attach_network(state, job, &mut config).await?;
    if job.service_kind == "application" {
        crate::gateway::attach(state, &mut config).await?;
    }
    if job.service_kind == "database" {
        super::services::database::configure(state, job, &mut config).await?;
    }
    let name = format!("ferrix-{}-{}", job.application_id.simple(), job.id.simple());
    let created = state
        .docker
        .create_container(
            Some(CreateContainerOptionsBuilder::default().name(&name).build()),
            config,
        )
        .await
        .map_err(
            |_| "Could not create the container. Check Docker resources and image configuration.",
        )?;
    sqlx::query("UPDATE deployments SET container_id=$1 WHERE id=$2 AND status='running'")
        .bind(&created.id)
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not persist container ID.")?;
    if job.service_kind == "database" {
        phase(
            conn,
            job.id,
            "Stopping the previous database before opening its persistent volume…",
        )
        .await?;
        super::services::database::stop_previous(state, conn, job).await?;
    }
    phase(conn, job.id, "Starting container…").await?;
    state
        .docker
        .start_container(&created.id, None)
        .await
        .map_err(
            |_| "Could not start the container. Inspect the image entrypoint and Docker resources.",
        )?;
    // A running state is required twice. Image-defined healthchecks must also become healthy.
    let mut ready = 0;
    let host_port = loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let info = inspect_owned(state, job.application_id, &created.id)
            .await
            .map_err(|_| "Could not verify container ownership and state.")?;
        let status = info
            .state
            .ok_or("Docker did not return a container state.")?;
        if status.running != Some(true) {
            return Err(
                "Container exited during startup. Check its image configuration and runtime logs."
                    .into(),
            );
        }
        let healthy = status.health.and_then(|h| h.status).map(|s| s.to_string());
        if healthy.as_deref() == Some("unhealthy") {
            return Err("Container healthcheck failed.".into());
        }
        if healthy.is_none() || healthy.as_deref() == Some("healthy") {
            ready += 1;
        } else {
            ready = 0;
        }
        if ready >= 2 {
            if job.service_kind == "database" {
                break None;
            }
            let port = info
                .network_settings
                .and_then(|n| n.ports)
                .and_then(|mut p| p.remove(&port_key))
                .flatten()
                .and_then(|p| p.into_iter().next())
                .and_then(|p| p.host_port)
                .and_then(|p| p.parse::<i32>().ok())
                .ok_or("Docker did not allocate a host port.")?;
            break Some(port);
        }
    };
    phase(conn, job.id, "Container is running. Activating deployment…").await?;
    // Commit the active pointer and success together. Cleanup only happens after this transaction.
    let mut tx = conn
        .begin()
        .await
        .map_err(|_| "Could not begin deployment activation.")?;
    let count=sqlx::query("UPDATE deployments SET status='succeeded',restore_container_id=NULL,finished_at=now(),logs=logs || E'Container activated successfully.\\n' WHERE id=$1 AND status='running'")
        .bind(job.id).execute(&mut *tx).await.map_err(|_|"Could not persist deployment success.")?.rows_affected();
    if count != 1 {
        return Err("Deployment was interrupted before activation.".into());
    }
    sqlx::query("UPDATE applications SET active_container_id=$1,active_deployment_id=$2,host_port=$3 WHERE id=$4")
        .bind(&created.id).bind(job.id).bind(host_port).bind(job.application_id).execute(&mut *tx).await.map_err(|_|"Could not activate deployment.")?;
    tx.commit().await.map_err(
        |_| "Could not confirm deployment activation. Recovery will reconcile its state.",
    )?;
    Ok(())
}
use sqlx::Connection;
// Only containers with matching Ferrix labels AND an existing terminal deployment in this database
// are eligible for cleanup. Other users' containers and the application's active container stay intact.
async fn cleanup(state: &AppState, conn: &mut PgConnection) -> Result<(), String> {
    let options = ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&HashMap::from([(
            "label".to_owned(),
            vec!["io.ferrix.managed=true".to_owned()],
        )]))
        .build();
    let containers = tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.list_containers(Some(options)),
    )
    .await
    .map_err(|_| "Docker cleanup timed out.")?
    .map_err(|_| "Docker cleanup unavailable.")?;
    for container in containers {
        let Some(labels) = container.labels else {
            continue;
        };
        let Some(deployment) = labels
            .get("io.ferrix.deployment")
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let Some(app) = labels
            .get("io.ferrix.application")
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let Some(container_id) = container.id else {
            continue;
        };
        let removable:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments d JOIN applications a ON a.id=d.application_id WHERE d.id=$1 AND a.id=$2 AND a.service_kind IN ('application','database') AND d.status IN ('succeeded','failed') AND a.active_container_id IS DISTINCT FROM $3)")
            .bind(deployment).bind(app).bind(&container_id).fetch_one(&mut *conn).await.map_err(|_|"Could not verify cleanup ownership.")?;
        if removable && labels_match(Some(&labels), app) {
            let removed = tokio::time::timeout(
                Duration::from_secs(10),
                state.docker.remove_container(
                    &container_id,
                    Some(
                        RemoveContainerOptionsBuilder::default()
                            .force(true)
                            .v(false)
                            .build(),
                    ),
                ),
            )
            .await;
            match removed {
                Ok(Ok(()))
                | Ok(Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404,
                    ..
                })) => {}
                _ => return Err(
                    "Could not clean the previous candidate safely. Database recovery will retry."
                        .into(),
                ),
            }
        }
    }
    Ok(())
}
async fn cycle(
    state: &AppState,
    conn: &mut PgConnection,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<bool, String> {
    // The exclusive session lock guarantees that no other live worker owns these running jobs.
    sqlx::query("UPDATE deployments SET status='failed',finished_at=now(),logs=logs || E'Deployment interrupted by worker restart. Redeploy to try again.\\n' WHERE status='running'")
        .execute(&mut *conn).await.map_err(|_|"Could not recover interrupted deployments.")?;
    cleanup(state, conn).await?;
    super::services::database::restore_pending(state, conn).await?;
    if super::backups::cycle(state, conn, shutdown).await? {
        return Ok(true);
    }
    let job=sqlx::query_as::<_,Job>("WITH next AS (SELECT id FROM deployments WHERE status='queued' ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1), claimed AS (UPDATE deployments d SET status='running',started_at=now() FROM next WHERE d.id=next.id RETURNING d.*) SELECT d.id,d.application_id,d.image,d.pinned_image_id,d.container_port,d.env_cipher,d.service_kind,d.service_cipher,p.organization_id,p.id AS project_id,e.id AS environment_id FROM claimed d JOIN applications a ON a.id=d.application_id JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id")
        .fetch_optional(&mut *conn).await.map_err(|_|"Could not claim queued deployment.")?;
    if let Some(job) = job {
        let authorized:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments d LEFT JOIN auth_users u ON u.id=d.requested_by LEFT JOIN auth_members m ON m.user_id=u.id AND m.organization_id=$2 WHERE d.id=$1 AND (d.requested_by IS NULL OR (NOT u.banned AND m.role IN ('owner','admin') AND d.service_kind IN ('application','database'))))")
            .bind(job.id).bind(job.organization_id).fetch_one(&mut *conn).await.map_err(|_|"Could not verify deployment authorization.")?;
        let result = if authorized {
            tokio::select! {
                result=tokio::time::timeout(Duration::from_secs(300),execute(state,conn,&job))=>result.unwrap_or_else(|_|Err("Deployment timed out after five minutes. The previous deployment is preserved.".into())),
                _=shutdown.changed()=>Err("Deployment interrupted during server shutdown. Redeploy to try again.".into()),
            }
        } else {
            Err("The requesting user no longer has permission to deploy this application.".into())
        };
        if let Err(error) = result {
            sqlx::query("UPDATE deployments SET status='failed',finished_at=now(),logs=left(logs || $1 || E'\\n',65536) WHERE id=$2 AND status='running'")
                .bind(error).bind(job.id).execute(&mut *conn).await.map_err(|_|"Could not record deployment failure.")?;
        }
        cleanup(state, conn).await?;
        super::services::database::restore_pending(state, conn).await?;
        Ok(true)
    } else {
        cleanup(state, conn).await?;
        Ok(false)
    }
}
pub async fn run_worker(state: AppState, mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        let result=async {
            let mut conn=state.pool.acquire().await.map_err(|_|"Deployment worker could not connect to PostgreSQL.")?;
            // Schema-specific key also isolates integration tests from development workers.
            let locked:bool=sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended(current_database() || ':' || current_schema() || ':ferrix-deploy',0))")
                .fetch_one(&mut *conn).await.map_err(|_|"Could not acquire deployment worker lock.")?;
            if !locked {return Ok::<_,String>(false);}
            let result=cycle(&state,&mut conn,&mut shutdown).await;
            // Never return a locked session to the pool if unlock fails.
            if sqlx::query("SELECT pg_advisory_unlock(hashtextextended(current_database() || ':' || current_schema() || ':ferrix-deploy',0))").execute(&mut *conn).await.is_err() {
                let _=conn.close().await;
            }
            result
        }.await;
        if let Err(error) = &result {
            tracing::warn!(%error,"Deployment worker retrying");
        }
        if *shutdown.borrow() {
            return;
        }
        if matches!(result, Ok(true)) {
            continue;
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(2))=>{},_=shutdown.changed()=>{return;}}
    }
}

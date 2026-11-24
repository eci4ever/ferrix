use super::*;
use bollard::query_parameters::{
    ListContainersOptionsBuilder, RestartContainerOptionsBuilder, StopContainerOptionsBuilder,
};
use futures_util::{StreamExt, TryStreamExt};
use std::time::Duration;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Start,
    Stop,
    Reload,
}
#[derive(Deserialize)]
pub(super) struct Control {
    operation: Operation,
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Could not complete the service operation. Check runtime and Docker before retrying.",
    )
}

pub(super) async fn change(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<Control>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    let mut tx = state.pool.begin().await?;
    // Serialize lifecycle operations with deployment snapshots and deletion.
    let app = sqlx::query_as::<_,Application>(&format!("SELECT {APP_COLUMNS} FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2 AND a.service_kind IN ('application','database') FOR UPDATE OF a"))
        .bind(service).bind(id.organization_id).fetch_optional(&mut *tx).await?.ok_or_else(missing)?;
    project_bundles::lock_app(&mut tx, service, id.organization_id).await?;
    super::super::backups::ensure_idle(&mut tx, service).await?;
    if !matches!(input.operation, Operation::Stop) {
        super::super::backups::ensure_restored(&mut tx, service).await?;
    }
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND (status IN ('queued','running') OR restore_container_id IS NOT NULL))").bind(service).fetch_one(&mut *tx).await?;
    if pending {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Wait for the pending deployment to finish before changing runtime state.",
        ));
    }
    let application = app.kind == "application";
    let containers: Vec<_> = app.active_container_id.into_iter().collect();
    if containers.is_empty() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Deploy this service before using runtime controls.",
        ));
    }
    if application {
        super::super::application_runtime::lock_lineage(&mut tx, service).await?;
        if matches!(input.operation, Operation::Start) {
            let container = &containers[0];
            let info = super::super::worker::inspect_owned(&state, service, container).await?;
            if info.state.and_then(|s| s.running) != Some(true) {
                let cipher:Option<Vec<u8>>=sqlx::query_scalar("SELECT service_cipher FROM deployments WHERE id=(SELECT active_deployment_id FROM applications WHERE id=$1)").bind(service).fetch_one(&mut *tx).await?;
                let runtime =
                    super::super::application_runtime::decode(&state, service, cipher.as_deref())?;
                super::super::application_runtime::ensure_single_writer(&state,service,&runtime,&mut tx).await.map_err(|_|ApiError(StatusCode::CONFLICT,"Another related service is running or recovering, or its storage is in use. Stop it before starting this service."))?;
            }
        }
    }
    tokio::time::timeout(
        Duration::from_secs(90),
        futures_util::stream::iter(containers)
            .map(|container| {
                let state = &state;
                async move {
                    super::super::worker::inspect_owned(state, service, &container).await?;
                    let result = match input.operation {
                        Operation::Start => {
                            if application {state.docker.update_container(&container,bollard::models::ContainerUpdateBody{restart_policy:Some(bollard::models::RestartPolicy{name:Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),maximum_retry_count:None}),..Default::default()}).await.map_err(|_|unavailable())?;}
                            state
                                .docker
                                .start_container(
                                    &container,
                                    None::<bollard::query_parameters::StartContainerOptions>,
                                )
                                .await
                        }
                        Operation::Stop => {
                            state
                                .docker
                                .stop_container(
                                    &container,
                                    Some(StopContainerOptionsBuilder::default().t(10).build()),
                                )
                                .await
                        }
                        Operation::Reload => {
                            state
                                .docker
                                .restart_container(
                                    &container,
                                    Some(RestartContainerOptionsBuilder::default().t(10).build()),
                                )
                                .await
                        }
                    };
                    match result {
                        Ok(())
                        | Err(bollard::errors::Error::DockerResponseServerError {
                            status_code: 304,
                            ..
                        }) => Ok(()),
                        Err(_) => Err(unavailable()),
                    }
                }
            })
            .buffer_unordered(4)
            .try_collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| unavailable())??;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// One scoped Docker inventory per project environment, rather than one probe per card.
pub(super) async fn statuses(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    environment_access(&state, &id, environment).await?;
    let apps: Vec<Application> = sqlx::query_as(&format!(
        "SELECT {APP_COLUMNS} FROM applications a WHERE environment_id=$1 AND service_kind IN ('application','database')"
    ))
    .bind(environment)
    .fetch_all(&state.pool)
    .await?;
    let filters = HashMap::from([(
        "label".to_owned(),
        vec![
            format!("io.ferrix.environment={environment}"),
            "io.ferrix.managed=true".into(),
        ],
    )]);
    let inventory = tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filters)
                .build(),
        )),
    )
    .await;
    let rows =
        apps.into_iter()
            .map(|app| {
                let runtime =
                    match &inventory {
                        Ok(Ok(containers)) => {
                            let owned = containers
                                .iter()
                                .filter(|c| {
                                    let labels = c.labels.as_ref();
                                    labels.and_then(|l| l.get("io.ferrix.application"))
                                        == Some(&app.id.to_string())
                                        && c.id.is_some()
                                        && c.id == app.active_container_id
                                })
                                .collect::<Vec<_>>();
                            if owned.is_empty() {
                                if app.deployed {
                                    "missing"
                                } else {
                                    "not_deployed"
                                }
                            } else if owned.iter().any(|c| {
                                c.status
                                    .as_deref()
                                    .is_some_and(|s| s.contains("(unhealthy)"))
                            }) {
                                "unhealthy"
                            } else if owned.iter().any(|c| {
                                c.status
                                    .as_deref()
                                    .is_some_and(|s| s.contains("(health: starting)"))
                            }) {
                                "starting"
                            } else if owned.iter().all(|c| {
                                c.state.as_ref().is_some_and(|s| s.to_string() == "running")
                            }) {
                                "running"
                            } else if owned.iter().any(|c| {
                                c.state.as_ref().is_some_and(|s| s.to_string() == "running")
                            }) {
                                "partially_running"
                            } else if owned.iter().any(|c| {
                                c.state
                                    .as_ref()
                                    .is_some_and(|s| s.to_string() == "restarting")
                            }) {
                                "restarting"
                            } else {
                                "stopped"
                            }
                        }
                        _ => "unavailable",
                    };
                json!({"id":app.id,"state":runtime})
            })
            .collect::<Vec<_>>();
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
pub(super) struct EnvironmentEdit {
    environment: String,
}
pub(super) async fn environment(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    app_access(&state, &id, service).await?;
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT env_cipher FROM applications WHERE id=$1")
            .bind(service)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(
        json!({"environment":decrypt(&state,service,cipher.as_deref())?.join("\n")}),
    ))
}
pub(super) async fn save_environment(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<EnvironmentEdit>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    let app = app_access(&state, &id, service).await?;
    let values = parse_environment(&input.environment)?;
    if app.kind == "database" {
        let config: DatabaseConfig = open(&state, service, &cipher(&state, service).await?)?;
        if values.iter().any(|v| {
            v.split_once('=')
                .is_some_and(|(key, _)| config.engine.reserved(key))
        }) {
            return Err(invalid(
                "Database credential and initialization variables are managed by Ferrix. Use General for credentials.",
            ));
        }
    }
    sqlx::query("UPDATE applications SET env_cipher=$1,updated_at=now() WHERE id=$2")
        .bind(encrypt(&state, service, &values)?)
        .bind(service)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

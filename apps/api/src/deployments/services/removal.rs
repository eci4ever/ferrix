use super::*;
use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListNetworksOptionsBuilder, ListVolumesOptionsBuilder,
    RemoveContainerOptionsBuilder, StopContainerOptionsBuilder,
};
use std::time::Duration;

#[derive(Deserialize)]
pub(super) struct Confirmation {
    name: String,
    #[serde(default)]
    remove_data: bool,
}
fn failure() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Could not finish Docker cleanup. The service record is retained; check Docker and retry deletion.",
    )
}
fn owned(labels: &HashMap<String, String>, service: Uuid) -> bool {
    labels.get("io.ferrix.application") == Some(&service.to_string())
        && labels.get("io.ferrix.managed").map(String::as_str) == Some("true")
}
fn tolerate_missing(result: Result<(), bollard::errors::Error>) -> Result<(), ApiError> {
    match result {
        Ok(())
        | Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => Ok(()),
        Err(_) => Err(failure()),
    }
}
pub(super) async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<Confirmation>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    app_access(&state, &id, service).await?;
    let mut tx = state.pool.begin().await?;
    let current: String =
        sqlx::query_scalar("SELECT name FROM applications WHERE id=$1 FOR UPDATE")
            .bind(service)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(missing)?;
    if input.name != current {
        return Err(invalid("Type the exact service name to confirm deletion."));
    }
    super::super::backups::ensure_idle(&mut tx, service).await?;
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND status IN ('queued','running'))")
        .bind(service).fetch_one(&mut *tx).await?;
    if pending {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Wait for the queued or running deployment to finish before deleting this service.",
        ));
    }
    // Holding the service row lock prevents new deployment snapshots during cleanup.
    tokio::time::timeout(
        Duration::from_secs(90),
        cleanup(&state, service, input.remove_data),
    )
    .await
    .map_err(|_| failure())??;
    sqlx::query("DELETE FROM applications WHERE id=$1")
        .bind(service)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn cleanup(state: &AppState, service: Uuid, remove_data: bool) -> Result<(), ApiError> {
    let filters = HashMap::from([(
        "label".to_owned(),
        vec![
            format!("io.ferrix.application={service}"),
            "io.ferrix.managed=true".into(),
        ],
    )]);
    let containers = state
        .docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filters)
                .build(),
        ))
        .await
        .map_err(|_| failure())?;
    for container in containers {
        if !container.labels.as_ref().is_some_and(|l| owned(l, service)) {
            return Err(failure());
        }
        let container = container.id.ok_or_else(failure)?;
        // Inspect again immediately before removal; never remove anonymous volumes implicitly.
        super::super::worker::inspect_owned(state, service, &container).await?;
        // Give stateful processes time to flush data before retaining their volume.
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
                status_code: 304 | 404,
                ..
            }) => {}
            Err(_) => return Err(failure()),
        }
        tolerate_missing(
            state
                .docker
                .remove_container(
                    &container,
                    Some(
                        RemoveContainerOptionsBuilder::default()
                            .force(true)
                            .v(false)
                            .build(),
                    ),
                )
                .await,
        )?;
    }
    let networks = state
        .docker
        .list_networks(Some(
            ListNetworksOptionsBuilder::default()
                .filters(&filters)
                .build(),
        ))
        .await
        .map_err(|_| failure())?;
    for network in networks {
        if !network.labels.as_ref().is_some_and(|l| owned(l, service)) {
            return Err(failure());
        }
        tolerate_missing(
            state
                .docker
                .remove_network(&network.id.ok_or_else(failure)?)
                .await,
        )?;
    }
    // Environment networks are shared by sibling services and deliberately retained.
    if remove_data {
        let volumes = state
            .docker
            .list_volumes(Some(
                ListVolumesOptionsBuilder::default()
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|_| failure())?;
        for volume in volumes.volumes.unwrap_or_default() {
            if !owned(&volume.labels, service) {
                return Err(failure());
            }
            // Docker refuses volumes still attached to another container (no force).
            tolerate_missing(
                state
                    .docker
                    .remove_volume(
                        &volume.name,
                        None::<bollard::query_parameters::RemoveVolumeOptions>,
                    )
                    .await,
            )?;
        }
    }
    Ok(())
}

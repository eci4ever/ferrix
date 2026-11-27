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
    project_bundles::lock_app(&mut tx, service, id.organization_id).await?;
    let shared: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_databases WHERE service_id=$1) OR EXISTS(SELECT 1 FROM php_classroom_applications WHERE database_service_id=$1)")
            .bind(service)
            .fetch_one(&mut *tx)
            .await?;
    if shared {
        return Err(invalid(
            "This shared MariaDB service contains managed logical databases. Retain it until their data and accounts have been explicitly removed.",
        ));
    }
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
// The worker may retire a container after the deletion inventory is collected.
async fn remove_listed_container(
    state: &AppState,
    service: Uuid,
    container: &str,
) -> Result<(), ApiError> {
    // Inspect again immediately before removal; never remove anonymous volumes implicitly.
    let info = match tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.inspect_container(container, None),
    )
    .await
    .map_err(|_| failure())?
    {
        Ok(info) => info,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => return Ok(()),
        Err(_) => return Err(failure()),
    };
    if !info
        .config
        .as_ref()
        .and_then(|c| c.labels.as_ref())
        .is_some_and(|labels| owned(labels, service))
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Container ownership could not be verified.",
        ));
    }
    // Give stateful processes time to flush data before retaining their volume.
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
            status_code: 304 | 404,
            ..
        }) => {}
        Err(_) => return Err(failure()),
    }
    tolerate_missing(
        state
            .docker
            .remove_container(
                container,
                Some(
                    RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .v(false)
                        .build(),
                ),
            )
            .await,
    )?;
    Ok(())
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
        remove_listed_container(state, service, &container).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker with caddy:2-alpine"]
    async fn deletion_tolerates_container_retired_after_inventory() {
        let fixture = super::super::super::test_support::Fixture::new().await;
        let container = fixture
            .state
            .docker
            .create_container(
                None::<bollard::query_parameters::CreateContainerOptions>,
                bollard::models::ContainerCreateBody {
                    image: Some("caddy:2-alpine".into()),
                    labels: Some(HashMap::from([
                        ("io.ferrix.managed".into(), "true".into()),
                        ("io.ferrix.application".into(), fixture.service.to_string()),
                    ])),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
        let foreign = remove_listed_container(&fixture.state, Uuid::new_v4(), &container).await;
        let preserved = fixture
            .state
            .docker
            .inspect_container(&container, None)
            .await
            .is_ok();
        // A real worker can remove a previously listed retired candidate at this point.
        fixture
            .state
            .docker
            .remove_container(&container, None)
            .await
            .unwrap();
        let result = remove_listed_container(&fixture.state, fixture.service, &container).await;
        fixture.finish().await;
        assert!(matches!(foreign, Err(ApiError(StatusCode::CONFLICT, _))));
        assert!(
            preserved,
            "Ownership mismatch must leave the container intact"
        );
        assert!(
            result.is_ok(),
            "A container already removed by the worker must not fail deletion"
        );
    }
}

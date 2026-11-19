use crate::{ApiError, AppState, auth::Identity};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{StatusCode, header},
};
use bollard::{Docker, query_parameters::EventsOptionsBuilder};
use futures_util::{StreamExt, TryStreamExt};
use serde_json::{Value, json};
use std::time::Duration;

pub(crate) async fn get(
    id: Identity,
    State(state): State<AppState>,
    Extension(monitor): Extension<crate::monitoring::Monitor>,
    Path(section): Path<String>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    id.admin()?;
    let data = if section == "disk-usage" {
        json!(crate::monitoring::cached_docker_usage(&monitor))
    } else {
        tokio::time::timeout(Duration::from_secs(5), inventory(&state.docker, &section))
            .await
            .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Docker request timed out."))??
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(data)))
}

// Return only fields needed for display. Docker replies can include secrets,
// mount paths, and arbitrary event attributes.
async fn inventory(docker: &Docker, section: &str) -> Result<Value, ApiError> {
    let unavailable = |_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Docker inventory unavailable. Try refreshing.",
        )
    };
    Ok(match section {
        "images" => json!(docker.list_images(None::<bollard::query_parameters::ListImagesOptions>).await.map_err(unavailable)?
            .into_iter().map(|image| json!({
                "id": image.id, "tags": image.repo_tags.join(", "), "size": image.size,
            })).collect::<Vec<_>>()),
        "volumes" => json!(docker.list_volumes(None::<bollard::query_parameters::ListVolumesOptions>).await.map_err(unavailable)?
            .volumes.unwrap_or_default().into_iter().map(|volume| json!({
                "id": volume.name, "driver": volume.driver, "scope": volume.scope,
            })).collect::<Vec<_>>()),
        "networks" => json!(docker.list_networks(None::<bollard::query_parameters::ListNetworksOptions>).await.map_err(unavailable)?
            .into_iter().map(|network| json!({
                "id": network.id, "name": network.name, "driver": network.driver, "scope": network.scope,
            })).collect::<Vec<_>>()),
        "swarm" => {
            let swarm = docker.info().await.map_err(unavailable)?.swarm.unwrap_or_default();
            json!({
                "state": swarm.local_node_state.map(|state| state.to_string()),
                "manager": swarm.control_available, "nodes": swarm.nodes, "managers": swarm.managers,
            })
        }
        "events" => {
            let now = chrono::Utc::now().timestamp();
            let options = EventsOptionsBuilder::default()
                .since(&(now - 900).to_string())
                .until(&now.to_string())
                .build();
            let mut events: Vec<_> = docker.events(Some(options)).take(256).try_collect().await.map_err(unavailable)?;
            events.reverse();
            json!(events.into_iter().enumerate().map(|(index, event)| json!({
                "id": index.to_string(),
                "type": event.typ.map(|kind| kind.to_string()),
                "action": event.action,
                "object": event.actor.and_then(|actor| actor.id),
                "time": event.time,
            })).collect::<Vec<_>>())
        }
        "health" => {
            docker.ping().await.map_err(unavailable)?;
            let version = docker.version().await.map_err(unavailable)?;
            json!({ "connected": true, "version": version.version, "api": version.api_version, "os": version.os, "arch": version.arch })
        }
        _ => return Err(ApiError(StatusCode::NOT_FOUND, "Unknown Docker section.")),
    })
}

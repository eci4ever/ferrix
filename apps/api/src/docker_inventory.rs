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
            let state = swarm.local_node_state.map(|state| state.to_string());
            let nodes = if state.as_deref() == Some("active") && swarm.control_available == Some(true) {
                Some(docker.list_nodes(None::<bollard::query_parameters::ListNodesOptions>).await.map_err(unavailable)?
                    .into_iter().map(|node| swarm_node(&json!(node))).collect::<Vec<_>>())
            } else { None };
            json!({
                "state": state,
                "manager": swarm.control_available, "nodes": swarm.nodes, "managers": swarm.managers,
                "node_id": swarm.node_id, "node_address": swarm.node_addr,
                "node_list": nodes,
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

// Deliberately exclude node labels, TLS certificates and manager addresses.
fn swarm_node(node: &Value) -> Value {
    let field = |path| node.pointer(path).cloned().unwrap_or(Value::Null);
    json!({
        "id": field("/ID"), "hostname": field("/Description/Hostname"),
        "role": field("/Spec/Role"), "availability": field("/Spec/Availability"),
        "state": field("/Status/State"), "address": field("/Status/Addr"),
        "leader": field("/ManagerStatus/Leader"), "reachability": field("/ManagerStatus/Reachability"),
        "engine_version": field("/Description/Engine/EngineVersion"),
        "nano_cpus": field("/Description/Resources/NanoCPUs"),
        "memory_bytes": field("/Description/Resources/MemoryBytes"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn swarm_inventory_handles_manager_worker_and_inactive_daemons() {
        for (state, manager) in [("active", true), ("active", false), ("inactive", false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let app = axum::Router::new().fallback(move |uri: axum::http::Uri| async move {
                if uri.path().ends_with("/info") {
                    Json(json!({"Swarm": {"LocalNodeState": state, "ControlAvailable": manager, "NodeID": "local"}}))
                } else {
                    assert!(manager && state == "active", "worker/inactive must not query nodes");
                    assert!(uri.path().ends_with("/nodes"));
                    Json(json!([{"ID": "local", "Description": {"Hostname": "vps"}, "Spec": {"Role": "manager"}, "Status": {"State": "ready"}}]))
                }
            });
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let docker =
                Docker::connect_with_http(&endpoint, 3, bollard::API_DEFAULT_VERSION).unwrap();
            let result = inventory(&docker, "swarm").await.unwrap();
            assert_eq!(result["state"], state);
            if manager {
                assert_eq!(result["node_list"][0]["hostname"], "vps");
            } else {
                assert!(result["node_list"].is_null());
            }
            server.abort();
        }
    }

    #[test]
    fn node_projection_preserves_capacity_without_exposing_metadata() {
        let node = swarm_node(&json!({
            "ID": "node-1", "Spec": {"Role": "manager", "Labels": {"secret": "private"}},
            "Description": {"Hostname": "vps", "Resources": {"NanoCPUs": 2000000000_i64, "MemoryBytes": 4096}, "TLSInfo": {"TrustRoot": "certificate"}},
            "Status": {"State": "ready"}, "ManagerStatus": {"Leader": true}
        }));
        assert_eq!(node["nano_cpus"], 2000000000_i64);
        assert_eq!(node["state"], "ready");
        assert_eq!(node["leader"], true);
        assert!(node["address"].is_null());
        assert!(!node.to_string().contains("private"));
        assert!(!node.to_string().contains("certificate"));
    }
}

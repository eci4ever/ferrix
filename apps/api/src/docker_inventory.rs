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

const SWARM_LOG_LIMIT: usize = 65536;

pub(crate) async fn logs(
    id: Identity,
    State(state): State<AppState>,
    Path(service_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    id.admin()?;
    if service_id.len() != 25
        || !service_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Use a valid Swarm service ID.",
        ));
    }
    let data = tokio::time::timeout(
        Duration::from_secs(5),
        service_logs(&state.docker, &service_id),
    )
    .await
    .map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Reading Swarm logs timed out. Refresh logs to retry.",
        )
    })??;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(data)))
}

fn log_error(error: bollard::errors::Error) -> ApiError {
    if matches!(
        error,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    ) {
        ApiError(
            StatusCode::NOT_FOUND,
            "Swarm service no longer exists. Refresh swarm.",
        )
    } else {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Docker could not read Swarm logs. Check the Docker connection and whether the service logging driver supports reading logs, then refresh logs.",
        )
    }
}

async fn service_logs(docker: &Docker, service_id: &str) -> Result<Value, ApiError> {
    let swarm = docker
        .info()
        .await
        .map_err(log_error)?
        .swarm
        .unwrap_or_default();
    if swarm
        .local_node_state
        .map(|state| state.to_string())
        .as_deref()
        != Some("active")
        || swarm.control_available != Some(true)
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Reading service logs requires an active Swarm manager.",
        ));
    }
    let options = bollard::query_parameters::LogsOptionsBuilder::default()
        .stdout(true)
        .stderr(true)
        .follow(false)
        .timestamps(true)
        .tail("200")
        .build();
    let mut stream = docker.service_logs(service_id, Some(options));
    let mut output = Vec::new();
    let mut truncated = false;
    let mut received = 0_usize;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(log_error)?.into_bytes();
        received = received.saturating_add(bytes.len());
        if output.len().saturating_add(bytes.len()) > SWARM_LOG_LIMIT {
            truncated = true;
            let discard = output
                .len()
                .saturating_add(bytes.len())
                .saturating_sub(SWARM_LOG_LIMIT)
                .min(output.len());
            output.drain(..discard);
        }
        output.extend_from_slice(&bytes[bytes.len().saturating_sub(SWARM_LOG_LIMIT)..]);
        if received >= 1048576 {
            truncated = true;
            break;
        }
    }
    let text = String::from_utf8_lossy(&output);
    let mut start = text.len().saturating_sub(SWARM_LOG_LIMIT);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    Ok(
        json!({"logs": &text[start..], "truncated": truncated || start > 0, "tail":200,"limit_bytes":SWARM_LOG_LIMIT,"checked_at":chrono::Utc::now(),"service_id":service_id}),
    )
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScaleRequest {
    replicas: i64,
    version: i32,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestartRequest {
    version: i32,
}

enum ServiceChange {
    Scale(i64),
    Restart,
}

pub(crate) async fn restart(
    id: Identity,
    State(state): State<AppState>,
    Path(service_id): Path<String>,
    Json(input): Json<RestartRequest>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    if input.version < 1
        || service_id.len() != 25
        || !service_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Use a service ID and a valid version.",
        ));
    }
    tokio::time::timeout(Duration::from_secs(8), change_service(&state.docker, &service_id, input.version, ServiceChange::Restart))
        .await.map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Restart request timed out. Refresh swarm to check whether it was applied before retrying."))??;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn scale(
    id: Identity,
    State(state): State<AppState>,
    Path(service_id): Path<String>,
    Json(input): Json<ScaleRequest>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    if !(0..=100).contains(&input.replicas)
        || input.version < 1
        || service_id.len() != 25
        || !service_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Use a service ID, a valid version and 0–100 replicas.",
        ));
    }
    tokio::time::timeout(Duration::from_secs(8), scale_service(&state.docker, &service_id, input))
        .await.map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Scale request timed out. Refresh swarm to check whether it was applied before retrying."))??;
    Ok(StatusCode::NO_CONTENT)
}

fn service_update_error(error: bollard::errors::Error) -> ApiError {
    match error {
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        } => ApiError(
            StatusCode::NOT_FOUND,
            "Swarm service no longer exists. Refresh swarm.",
        ),
        bollard::errors::Error::DockerResponseServerError {
            status_code: 409, ..
        } => ApiError(
            StatusCode::CONFLICT,
            "Service changed. Refresh swarm before updating it again.",
        ),
        _ => ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Docker could not update this service. Refresh swarm to check its state before retrying.",
        ),
    }
}

async fn scale_service(
    docker: &Docker,
    service_id: &str,
    input: ScaleRequest,
) -> Result<(), ApiError> {
    change_service(
        docker,
        service_id,
        input.version,
        ServiceChange::Scale(input.replicas),
    )
    .await
}

async fn change_service(
    docker: &Docker,
    service_id: &str,
    version: i32,
    change: ServiceChange,
) -> Result<(), ApiError> {
    let swarm = docker
        .info()
        .await
        .map_err(service_update_error)?
        .swarm
        .unwrap_or_default();
    if swarm
        .local_node_state
        .map(|state| state.to_string())
        .as_deref()
        != Some("active")
        || swarm.control_available != Some(true)
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Updating services requires an active Swarm manager.",
        ));
    }
    let service = docker
        .inspect_service(service_id, None)
        .await
        .map_err(service_update_error)?;
    if service.version.and_then(|version| version.index) != Some(version as u64) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Service changed. Refresh swarm before updating it again.",
        ));
    }
    let mut spec = service.spec.ok_or(ApiError(
        StatusCode::CONFLICT,
        "Service configuration unavailable. Refresh swarm.",
    ))?;
    let mode = spec.mode.as_mut().ok_or(ApiError(
        StatusCode::BAD_REQUEST,
        "Service mode unavailable. Refresh swarm.",
    ))?;
    if mode.global_job.is_some() || mode.replicated_job.is_some() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Job services cannot be scaled or restarted here.",
        ));
    }
    match change {
        ServiceChange::Scale(replicas) => {
            let replicated = mode
                .replicated
                .as_mut()
                .filter(|_| mode.global.is_none())
                .ok_or(ApiError(
                    StatusCode::BAD_REQUEST,
                    "Only Replicated services can be scaled.",
                ))?;
            replicated.replicas = Some(replicas);
        }
        ServiceChange::Restart => {
            if mode.replicated.is_none() && mode.global.is_none() {
                return Err(ApiError(
                    StatusCode::BAD_REQUEST,
                    "Only Replicated and Global services can be restarted.",
                ));
            }
            let task = spec.task_template.as_mut().ok_or(ApiError(
                StatusCode::CONFLICT,
                "Service task configuration unavailable. Refresh swarm.",
            ))?;
            task.force_update = Some(task.force_update.unwrap_or(0).checked_add(1).ok_or(
                ApiError(
                    StatusCode::CONFLICT,
                    "Service restart counter is exhausted.",
                ),
            )?);
        }
    }
    docker
        .update_service(
            service_id,
            spec,
            bollard::query_parameters::UpdateServiceOptionsBuilder::default()
                .version(version)
                .registry_auth_from("spec")
                .build(),
            None,
        )
        .await
        .map_err(service_update_error)?;
    Ok(())
}

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
            let (nodes, services) = if state.as_deref() == Some("active") && swarm.control_available == Some(true) {
                let (nodes, services) = tokio::try_join!(
                    docker.list_nodes(None::<bollard::query_parameters::ListNodesOptions>),
                    docker.list_services(Some(bollard::query_parameters::ListServicesOptionsBuilder::default().status(true).build())),
                ).map_err(unavailable)?;
                (Some(nodes.into_iter().map(|node| swarm_node(&json!(node))).collect::<Vec<_>>()),
                 Some(services.into_iter().map(|service| swarm_service(&json!(service))).collect::<Vec<_>>()))
            } else { (None, None) };
            json!({
                "state": state,
                "manager": swarm.control_available, "nodes": swarm.nodes, "managers": swarm.managers,
                "node_id": swarm.node_id, "node_address": swarm.node_addr,
                "node_list": nodes,
                "service_list": services,
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

// Service specs may contain environment credentials, secrets, mounts and labels.
// Never return the raw spec or update-status message to the browser.
fn swarm_service(service: &Value) -> Value {
    let field = |path| service.pointer(path).cloned().unwrap_or(Value::Null);
    let mode = ["Replicated", "Global", "ReplicatedJob", "GlobalJob"]
        .into_iter()
        .find(|mode| {
            service
                .pointer(&format!("/Spec/Mode/{mode}"))
                .is_some_and(|value| !value.is_null())
        });
    let ports =
        service
            .pointer("/Endpoint/Ports")
            .and_then(Value::as_array)
            .map(|ports| {
                ports.iter().map(|port| json!({
            "published": port.get("PublishedPort"), "target": port.get("TargetPort"),
            "protocol": port.get("Protocol"), "mode": port.get("PublishMode"),
        })).collect::<Vec<_>>()
            });
    json!({
        "id": field("/ID"), "name": field("/Spec/Name"), "mode": mode,
        "version": field("/Version/Index"), "replicas": field("/Spec/Mode/Replicated/Replicas"),
        "image": field("/Spec/TaskTemplate/ContainerSpec/Image"),
        "running_tasks": field("/ServiceStatus/RunningTasks"),
        "desired_tasks": field("/ServiceStatus/DesiredTasks"),
        "completed_tasks": field("/ServiceStatus/CompletedTasks"),
        "update_state": field("/UpdateStatus/State"), "ports": ports,
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
    async fn log_snapshots_decode_streams_bound_output_and_reject_unavailable_access() {
        use axum::response::IntoResponse;
        for scenario in [
            "logs", "empty", "large", "worker", "inactive", "deleted", "driver",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let app = axum::Router::new().fallback(move |uri: axum::http::Uri| async move {
                if uri.path().ends_with("/info") {
                    return Json(json!({"Swarm":{"LocalNodeState":if scenario == "inactive" {"inactive"} else {"active"},"ControlAvailable":scenario != "worker"}})).into_response();
                }
                assert!(!["worker", "inactive"].contains(&scenario));
                assert!(uri.path().ends_with("/logs"));
                let query = uri.query().unwrap_or_default();
                for option in ["stdout=true", "stderr=true", "follow=false", "timestamps=true", "tail=200"] { assert!(query.contains(option)); }
                if ["deleted", "driver"].contains(&scenario) {
                    return (if scenario == "deleted" {StatusCode::NOT_FOUND} else {StatusCode::INTERNAL_SERVER_ERROR}, Json(json!({"message":"private Docker detail"}))).into_response();
                }
                let messages = match scenario { "empty" => vec![], "large" => vec!["é".repeat(40000)], _ => vec!["2026-11-23T00:00:00Z stdout entry\n".into(), "2026-11-23T00:00:01Z stderr entry\n".into()] };
                let mut framed = Vec::new();
                for (index, message) in messages.iter().enumerate() {
                    framed.extend_from_slice(&[if index == 0 {1} else {2},0,0,0]);
                    framed.extend_from_slice(&(message.len() as u32).to_be_bytes());
                    framed.extend_from_slice(message.as_bytes());
                }
                ([(header::CONTENT_TYPE,"application/vnd.docker.raw-stream")], framed).into_response()
            });
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let docker =
                Docker::connect_with_http(&endpoint, 3, bollard::API_DEFAULT_VERSION).unwrap();
            let result = service_logs(&docker, "aaaaaaaaaaaaaaaaaaaaaaaaa").await;
            match scenario {
                "logs" => {
                    let data = result.unwrap();
                    assert!(data["logs"].as_str().unwrap().contains("stdout entry"));
                    assert!(data["logs"].as_str().unwrap().contains("stderr entry"));
                    assert_eq!(data["truncated"], false);
                }
                "empty" => assert_eq!(result.unwrap()["logs"], ""),
                "large" => {
                    let data = result.unwrap();
                    assert_eq!(data["truncated"], true);
                    assert!(data["logs"].as_str().unwrap().len() <= SWARM_LOG_LIMIT);
                }
                _ => {
                    let error = result.err().unwrap();
                    assert_eq!(
                        error.0,
                        match scenario {
                            "worker" | "inactive" => StatusCode::CONFLICT,
                            "deleted" => StatusCode::NOT_FOUND,
                            _ => StatusCode::SERVICE_UNAVAILABLE,
                        }
                    );
                    assert!(!error.1.contains("private"));
                }
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn restart_replaces_tasks_without_changing_replicas_or_update_policy() {
        for mode in ["replicated", "global", "job", "overflow"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let app = axum::Router::new().fallback(move |request: axum::extract::Request| async move {
                let uri = request.uri().clone();
                let body = if uri.path().ends_with("/info") {
                    json!({"Swarm":{"LocalNodeState":"active","ControlAvailable":true}})
                } else if uri.path().ends_with("/update") {
                    assert!(["replicated", "global"].contains(&mode));
                    assert!(uri.query().unwrap_or_default().contains("version=12"));
                    assert!(uri.query().unwrap_or_default().contains("registryAuthFrom=spec"));
                    let bytes = axum::body::to_bytes(request.into_body(), 65536).await.unwrap();
                    let actual: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(actual["TaskTemplate"]["ForceUpdate"], 8);
                    assert_eq!(actual["TaskTemplate"]["ContainerSpec"]["Env"][0], "KEEP=private");
                    assert_eq!(actual["UpdateConfig"]["Parallelism"], 1);
                    if mode == "replicated" { assert_eq!(actual["Mode"]["Replicated"]["Replicas"], 2); }
                    else { assert!(actual["Mode"].get("Global").is_some()); }
                    json!({"Warnings":[]})
                } else {
                    json!({"ID":"aaaaaaaaaaaaaaaaaaaaaaaaa","Version":{"Index":12},"Spec":{"Name":"demo","Mode":match mode {"global"=>json!({"Global":{}}),"job"=>json!({"ReplicatedJob":{}}),_=>json!({"Replicated":{"Replicas":2}})},"UpdateConfig":{"Parallelism":1},"TaskTemplate":{"ForceUpdate":if mode == "overflow" {u64::MAX} else {7},"ContainerSpec":{"Image":"nginx:alpine","Env":["KEEP=private"]}}}})
                };
                Json(body)
            });
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let docker =
                Docker::connect_with_http(&endpoint, 3, bollard::API_DEFAULT_VERSION).unwrap();
            let result = change_service(
                &docker,
                "aaaaaaaaaaaaaaaaaaaaaaaaa",
                12,
                ServiceChange::Restart,
            )
            .await;
            let expected = match mode {
                "job" => Some(StatusCode::BAD_REQUEST),
                "overflow" => Some(StatusCode::CONFLICT),
                _ => None,
            };
            assert_eq!(result.err().map(|error| error.0), expected, "{mode}");
            server.abort();
        }
    }

    #[tokio::test]
    async fn scaling_preserves_spec_and_rejects_worker_modes_and_stale_versions() {
        for scenario in [
            "success", "worker", "inactive", "global", "job", "stale", "deleted", "race",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let app = axum::Router::new().fallback(move |request: axum::extract::Request| async move {
                let uri = request.uri().clone();
                let payload = if uri.path().ends_with("/info") {
                    json!({"Swarm":{"LocalNodeState":if scenario == "inactive" {"inactive"} else {"active"},"ControlAvailable":scenario != "worker"}})
                } else if uri.path().ends_with("/update") {
                    assert_eq!(request.method(), axum::http::Method::POST);
                    assert!(uri.query().unwrap_or_default().contains("version=12"));
                    assert!(uri.query().unwrap_or_default().contains("registryAuthFrom=spec"));
                    let body = axum::body::to_bytes(request.into_body(), 65536).await.unwrap();
                    let actual: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(actual["Mode"]["Replicated"]["Replicas"], 3);
                    assert_eq!(actual["TaskTemplate"]["ContainerSpec"]["Image"], "nginx:alpine");
                    assert_eq!(actual["TaskTemplate"]["ContainerSpec"]["Env"][0], "KEEP=secret");
                    assert_eq!(actual["Labels"]["keep"], "yes");
                    if scenario == "race" { return (StatusCode::CONFLICT, Json(json!({"message":"out of sequence"}))); }
                    assert_eq!(scenario, "success");
                    json!({"Warnings":[]})
                } else {
                    assert!(!["inactive", "worker"].contains(&scenario));
                    if scenario == "deleted" { return (StatusCode::NOT_FOUND, Json(json!({"message":"missing"}))); }
                    json!({"ID":"aaaaaaaaaaaaaaaaaaaaaaaaa","Version":{"Index":if scenario == "stale" {13} else {12}},"Spec":{"Name":"demo","Labels":{"keep":"yes"},"Mode":if scenario == "global" {json!({"Global":{}})} else if scenario == "job" {json!({"ReplicatedJob":{}})} else {json!({"Replicated":{"Replicas":2}})},"TaskTemplate":{"ContainerSpec":{"Image":"nginx:alpine","Env":["KEEP=secret"]}}}})
                };
                (StatusCode::OK, Json(payload))
            });
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let docker =
                Docker::connect_with_http(&endpoint, 3, bollard::API_DEFAULT_VERSION).unwrap();
            let result = scale_service(
                &docker,
                "aaaaaaaaaaaaaaaaaaaaaaaaa",
                ScaleRequest {
                    replicas: 3,
                    version: 12,
                },
            )
            .await;
            let expected = match scenario {
                "success" => None,
                "global" | "job" => Some(StatusCode::BAD_REQUEST),
                "deleted" => Some(StatusCode::NOT_FOUND),
                _ => Some(StatusCode::CONFLICT),
            };
            assert_eq!(result.err().map(|error| error.0), expected, "{scenario}");
            server.abort();
        }
    }

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
                    if uri.path().ends_with("/nodes") {
                        Json(json!([{"ID": "local", "Description": {"Hostname": "vps"}, "Spec": {"Role": "manager"}, "Status": {"State": "ready"}}]))
                    } else {
                        assert!(uri.path().ends_with("/services"));
                        assert!(uri.query().unwrap_or_default().contains("status=true"));
                        Json(json!([{"ID": "web", "Spec": {"Name": "web", "Mode": {"Replicated": {"Replicas": 3}}}, "ServiceStatus": {"RunningTasks": 2, "DesiredTasks": 3}}]))
                    }
                }
            });
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let docker =
                Docker::connect_with_http(&endpoint, 3, bollard::API_DEFAULT_VERSION).unwrap();
            let result = inventory(&docker, "swarm").await.unwrap();
            assert_eq!(result["state"], state);
            if manager {
                assert_eq!(result["node_list"][0]["hostname"], "vps");
                assert_eq!(result["service_list"][0]["running_tasks"], 2);
                assert_eq!(result["service_list"][0]["desired_tasks"], 3);
            } else {
                assert!(result["node_list"].is_null());
                assert!(result["service_list"].is_null());
            }
            server.abort();
        }
    }

    #[test]
    fn service_projection_handles_modes_and_excludes_secrets() {
        for mode in ["Replicated", "Global", "ReplicatedJob", "GlobalJob"] {
            let mut raw = json!({"ID": "web", "Spec": {"Name": "web", "Mode": {}, "Labels": {"secret": "private"}, "TaskTemplate": {"ContainerSpec": {"Image": "nginx:alpine", "Env": ["PASSWORD=private"], "Secrets": [{"SecretID": "private"}]}}}, "Endpoint": {"Ports": [{"PublishedPort": 8080, "TargetPort": 80, "Protocol": "tcp", "PublishMode": "ingress"}]}, "UpdateStatus": {"State": "paused", "Message": "private"}});
            raw["Spec"]["Mode"][mode] = json!({});
            let service = swarm_service(&raw);
            assert_eq!(service["mode"], mode);
            assert_eq!(service["image"], "nginx:alpine");
            assert_eq!(service["ports"][0]["published"], 8080);
            assert_eq!(service["update_state"], "paused");
            assert!(service["running_tasks"].is_null());
            assert!(!service.to_string().contains("private"));
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

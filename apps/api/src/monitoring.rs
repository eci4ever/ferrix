//! Process-wide collectors: requests only read a bounded cache, never run OS or Docker probes.
use crate::{ApiError, auth::Identity};
use axum::{Extension, Json};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use sysinfo::{Disks, Networks, System};

const INTERVAL: Duration = Duration::from_secs(5);
const CAPACITY: usize = 180;

#[derive(Clone, Serialize)]
pub(crate) struct Sample {
    at: i64,
    cpu: Option<f32>,
    memory_used: u64,
    memory_total: u64,
    disk_used: Option<u64>,
    disk_total: Option<u64>,
    read_bps: Option<f64>,
    write_bps: Option<f64>,
    receive_bps: Option<f64>,
    transmit_bps: Option<f64>,
}
#[derive(Clone, Default, Serialize)]
pub(crate) struct DockerUsage {
    at: Option<i64>,
    images: Option<u64>,
    containers: Option<u64>,
    volumes: Option<u64>,
    build_cache: Option<u64>,
    error: Option<&'static str>,
}
#[derive(Clone, Serialize)]
pub(crate) struct Snapshot {
    host: String,
    os: String,
    scope: &'static str,
    interval_seconds: u64,
    samples: VecDeque<Sample>,
    docker: DockerUsage,
}
pub(crate) type Monitor = Arc<RwLock<Snapshot>>;

fn append(samples: &mut VecDeque<Sample>, sample: Sample) {
    if samples.len() == CAPACITY {
        samples.pop_front();
    }
    samples.push_back(sample);
}

// A new device or reset counter has no valid delta. Do not manufacture a boot-time spike.
fn rates(
    current: &HashMap<String, (u64, u64)>,
    previous: &HashMap<String, (u64, u64)>,
    elapsed: f64,
) -> Option<(f64, f64)> {
    let mut result = (0u64, 0u64);
    let mut found = false;
    for (name, &(a, b)) in current {
        if let Some(&(old_a, old_b)) = previous.get(name)
            && let (Some(a), Some(b)) = (a.checked_sub(old_a), b.checked_sub(old_b))
        {
            result.0 = result.0.saturating_add(a);
            result.1 = result.1.saturating_add(b);
            found = true;
        }
    }
    (found && elapsed > 0.0).then_some((result.0 as f64 / elapsed, result.1 as f64 / elapsed))
}

pub(crate) fn start() -> Monitor {
    static SHARED: std::sync::OnceLock<Monitor> = std::sync::OnceLock::new();
    SHARED.get_or_init(start_collector).clone()
}
pub(crate) fn latest() -> Option<serde_json::Value> {
    start()
        .read()
        .ok()?
        .samples
        .back()
        .and_then(|s| serde_json::to_value(s).ok())
}
fn start_collector() -> Monitor {
    let cache = Arc::new(RwLock::new(Snapshot {
        host: System::host_name().unwrap_or_else(|| "Unknown host".into()),
        os: System::name().unwrap_or_else(|| "Unknown OS".into()),
        scope: "API host OS view; when Ferrix is containerized this is its namespace, not physical-host or container-limit accounting",
        interval_seconds: INTERVAL.as_secs(),
        samples: VecDeque::with_capacity(CAPACITY),
        docker: DockerUsage::default(),
    }));
    let weak = Arc::downgrade(&cache);
    // One blocking collector only. A slow OS probe cannot block requests or spawn more probes.
    std::thread::spawn(move || {
        let mut system = System::new();
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let mut old_disk = HashMap::new();
        let mut old_net = HashMap::new();
        let mut previous = Instant::now();
        let mut warmed = false;
        loop {
            if weak.strong_count() == 0 {
                break;
            }
            system.refresh_cpu_usage();
            system.refresh_memory();
            disks.refresh(true);
            networks.refresh(true);
            let now = Instant::now();
            let elapsed = now.duration_since(previous).as_secs_f64();
            let mut disk_counters = HashMap::new();
            for disk in &disks {
                let usage = disk.usage();
                // Deduplicate multiple mounts of the same device, including APFS volumes.
                disk_counters
                    .entry(disk.name().to_string_lossy().into_owned())
                    .or_insert((usage.total_read_bytes, usage.total_written_bytes));
            }
            let net_counters = networks
                .iter()
                .filter(|(name, _)| *name != "lo" && *name != "lo0")
                .map(|(name, net)| {
                    (
                        name.clone(),
                        (net.total_received(), net.total_transmitted()),
                    )
                })
                .collect();
            let io = rates(&disk_counters, &old_disk, elapsed);
            let net = rates(&net_counters, &old_net, elapsed);
            // Root filesystem only: summing mounts double-counts shared/APFS capacity.
            let root = disks
                .iter()
                .find(|disk| disk.mount_point() == std::path::Path::new("/"));
            let sample = Sample {
                at: chrono::Utc::now().timestamp_millis(),
                cpu: warmed.then(|| system.global_cpu_usage()),
                memory_used: system.used_memory(),
                memory_total: system.total_memory(),
                disk_used: root
                    .map(|disk| disk.total_space().saturating_sub(disk.available_space())),
                disk_total: root.map(|disk| disk.total_space()),
                read_bps: io.map(|v| v.0),
                write_bps: io.map(|v| v.1),
                receive_bps: net.map(|v| v.0),
                transmit_bps: net.map(|v| v.1),
            };
            if let Some(cache) = weak.upgrade() {
                append(&mut cache.write().unwrap().samples, sample);
            }
            old_disk = disk_counters;
            old_net = net_counters;
            previous = now;
            warmed = true;
            std::thread::sleep(INTERVAL);
        }
    });
    let weak = Arc::downgrade(&cache);
    tokio::spawn(async move {
        let probe = DockerProbe::new(std::env::var("DOCKER_HOST").ok().as_deref());
        loop {
            if weak.strong_count() == 0 {
                break;
            }
            let usage = tokio::time::timeout(Duration::from_secs(3), async {
                match &probe {
                    Ok(probe) => probe.collect().await,
                    Err(error) => Err(*error),
                }
            })
            .await;
            let Some(cache) = weak.upgrade() else {
                break;
            };
            {
                let mut snapshot = cache.write().unwrap();
                match usage {
                    Ok(Ok(usage)) => snapshot.docker = docker_usage(usage),
                    _ => {
                        snapshot.docker.error = Some(
                            "Docker storage unavailable or timed out; any previous reading is stale.",
                        )
                    }
                }
            }
            drop(cache);
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    cache
}

// Bollard 0.20's generated df response loses the legacy fields still returned by
// Docker. Read this one endpoint directly, preserving all deployment SDK behavior.
struct DockerProbe {
    client: reqwest::Client,
    endpoint: String,
}
impl DockerProbe {
    fn new(host: Option<&str>) -> Result<Self, &'static str> {
        let host = host.unwrap_or("unix:///var/run/docker.sock");
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3));
        let endpoint = if let Some(socket) = host.strip_prefix("unix://") {
            if socket.is_empty() {
                return Err("Empty Docker socket");
            }
            #[cfg(unix)]
            {
                builder = builder.unix_socket(socket);
            }
            #[cfg(not(unix))]
            {
                return Err("Unix sockets unsupported");
            }
            "http://localhost/system/df".to_owned()
        } else {
            let host = host
                .strip_prefix("tcp://")
                .map(|h| format!("http://{h}"))
                .unwrap_or_else(|| host.to_owned());
            let mut url = url::Url::parse(&host).map_err(|_| "Invalid Docker host")?;
            if url.scheme() != "http"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err("Unsupported Docker endpoint");
            }
            url.set_path("/system/df");
            url.to_string()
        };
        Ok(Self {
            client: builder.build().map_err(|_| "Docker client unavailable")?,
            endpoint,
        })
    }
    async fn collect(&self) -> Result<serde_json::Value, &'static str> {
        let mut response = self
            .client
            .get(&self.endpoint)
            .send()
            .await
            .map_err(|_| "Docker request failed")?
            .error_for_status()
            .map_err(|_| "Docker rejected request")?;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Docker response failed")?
        {
            if body.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
                return Err("Docker response too large");
            }
            body.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| "Invalid Docker storage response")?;
        if value.get("LayersSize").is_none() && value.get("ImagesDiskUsage").is_none() {
            return Err("Unsupported Docker storage response");
        }
        Ok(value)
    }
}

pub(crate) async fn volume_inventory() -> Result<Vec<serde_json::Value>, &'static str> {
    let value = DockerProbe::new(std::env::var("DOCKER_HOST").ok().as_deref())?
        .collect()
        .await?;
    if let Some(v) = value.get("Volumes") {
        return if v.is_null() {
            Ok(Vec::new())
        } else {
            v.as_array().cloned().ok_or("Invalid volume inventory")
        };
    }
    if let Some(v) = value["VolumesDiskUsage"]["Items"].as_array() {
        return Ok(v.clone());
    }
    if value["VolumesDiskUsage"]["TotalCount"].as_i64() == Some(0) {
        return Ok(Vec::new());
    }
    Err("Volume usage data unavailable")
}

fn nonnegative(value: &serde_json::Value) -> Option<u64> {
    value.as_i64().and_then(|n| u64::try_from(n).ok())
}
fn sum_sizes(value: &serde_json::Value, field: &str) -> Option<u64> {
    if value.is_null() {
        return Some(0);
    }
    value.as_array()?.iter().try_fold(0u64, |sum, item| {
        sum.checked_add(nonnegative(&item[field])?)
    })
}
fn docker_usage(value: serde_json::Value) -> DockerUsage {
    if value.get("ImagesDiskUsage").is_some() {
        return DockerUsage {
            at: Some(chrono::Utc::now().timestamp_millis()),
            images: nonnegative(&value["ImagesDiskUsage"]["TotalSize"]),
            containers: nonnegative(&value["ContainersDiskUsage"]["TotalSize"]),
            volumes: nonnegative(&value["VolumesDiskUsage"]["TotalSize"]),
            build_cache: nonnegative(&value["BuildCacheDiskUsage"]["TotalSize"]),
            error: None,
        };
    }
    let volumes = if value["Volumes"].is_null() {
        Some(0)
    } else {
        value["Volumes"].as_array().and_then(|items| {
            items.iter().try_fold(0u64, |sum, v| {
                sum.checked_add(nonnegative(&v["UsageData"]["Size"])?)
            })
        })
    };
    DockerUsage {
        at: Some(chrono::Utc::now().timestamp_millis()),
        images: nonnegative(&value["LayersSize"]), // Docker's deduplicated image layers, not summed virtual sizes.
        containers: sum_sizes(&value["Containers"], "SizeRw"),
        volumes,
        build_cache: sum_sizes(&value["BuildCache"], "Size"),
        error: None,
    }
}

pub(crate) fn cached_docker_usage(cache: &Monitor) -> DockerUsage {
    cache.read().unwrap().docker.clone()
}

pub(crate) async fn get(
    id: Identity,
    Extension(cache): Extension<Monitor>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    id.admin()?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(cache.read().unwrap().clone()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn docker_endpoint_configuration() {
        assert_eq!(
            DockerProbe::new(Some("tcp://127.0.0.1:2375"))
                .unwrap()
                .endpoint,
            "http://127.0.0.1:2375/system/df"
        );
        assert_eq!(
            DockerProbe::new(Some("unix:///tmp/test.sock"))
                .unwrap()
                .endpoint,
            "http://localhost/system/df"
        );
        assert!(DockerProbe::new(Some("http://host/?query=bad")).is_err());
        assert!(DockerProbe::new(Some("ssh://host")).is_err());
    }
    #[tokio::test]
    async fn docker_raw_response_retains_storage_fields() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("tcp://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /system/df "));
            let body = r#"{"LayersSize":100,"Containers":[{"SizeRw":20}],"Volumes":[{"UsageData":{"Size":30}}],"BuildCache":[{"Size":40}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let usage = docker_usage(
            DockerProbe::new(Some(&endpoint))
                .unwrap()
                .collect()
                .await
                .unwrap(),
        );
        assert_eq!(
            (
                usage.images,
                usage.containers,
                usage.volumes,
                usage.build_cache
            ),
            (Some(100), Some(20), Some(30), Some(40))
        );
        server.await.unwrap();
    }
    #[test]
    fn history_evicts_oldest_samples() {
        let sample = Sample {
            at: 0,
            cpu: None,
            memory_used: 1,
            memory_total: 2,
            disk_used: None,
            disk_total: None,
            read_bps: None,
            write_bps: None,
            receive_bps: None,
            transmit_bps: None,
        };
        let mut history = VecDeque::new();
        for at in 0..CAPACITY + 15 {
            append(
                &mut history,
                Sample {
                    at: at as i64,
                    ..sample.clone()
                },
            );
        }
        assert_eq!(history.len(), CAPACITY);
        assert_eq!(history.front().unwrap().at, 15);
        assert_eq!(history.back().unwrap().at, (CAPACITY + 14) as i64);
    }
    #[test]
    fn docker_empty_optional_collections_are_zero() {
        let usage = docker_usage(serde_json::json!({"LayersSize":0}));
        assert_eq!(usage.images, Some(0));
        assert_eq!(usage.containers, Some(0));
        assert_eq!(usage.volumes, Some(0));
        assert_eq!(usage.build_cache, Some(0));
    }
    #[test]
    fn rates_handle_resets_new_devices_and_real_elapsed_time() {
        let old = HashMap::from([("disk".into(), (100, 200))]);
        let new = HashMap::from([("disk".into(), (300, 600)), ("new".into(), (9999, 9999))]);
        assert_eq!(rates(&new, &old, 2.0), Some((100.0, 200.0)));
        assert_eq!(rates(&old, &new, 2.0), None);
        assert_eq!(rates(&new, &old, 0.0), None);
    }
    #[test]
    fn docker_does_not_sum_shared_image_sizes_or_unknown_volumes() {
        let usage = docker_usage(
            serde_json::json!({"LayersSize":100,"Images":[{"Size":100},{"Size":100}],"Containers":[{"SizeRw":10}],"Volumes":[{"UsageData":{"Size":-1}}],"BuildCache":[]}),
        );
        assert_eq!(usage.images, Some(100));
        assert_eq!(usage.containers, Some(10));
        assert_eq!(usage.volumes, None);
        assert_eq!(usage.build_cache, Some(0));
    }
}

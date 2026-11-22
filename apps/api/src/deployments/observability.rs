use super::*;
use axum::Extension;
use bollard::{
    models::{ContainerStatsResponse, HostConfig},
    query_parameters::{LogsOptionsBuilder, StatsOptionsBuilder},
};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/metrics", get(metrics))
        .layer(Extension(MetricsCache::default()))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::identify,
        ))
}
const LOG_LIMIT: usize = 65536;
#[derive(Deserialize)]
pub(super) struct LogQuery {
    #[serde(default = "default_tail")]
    tail: u16,
    #[serde(default)]
    timestamps: bool,
}
fn default_tail() -> u16 {
    100
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Could not read service telemetry. Check the container and Docker connection.",
    )
}

pub(super) async fn logs(
    state: &AppState,
    app: &Application,
    input: LogQuery,
) -> Result<Value, ApiError> {
    if input.tail == 0 || input.tail > 1000 {
        return Err(invalid("Choose between 1 and 1000 log lines."));
    }
    let read = async {
        let mut output = Vec::new();
        let mut truncated = false;
        if let Some(container) = &app.active_container_id {
            worker::inspect_owned(state, app.id, container).await?;
            let options = LogsOptionsBuilder::default()
                .stdout(true)
                .stderr(true)
                .tail(&input.tail.to_string())
                .timestamps(input.timestamps)
                .build();
            let mut stream = state.docker.logs(container, Some(options));
            while let Some(line) = stream.next().await {
                let bytes = line.map_err(|_| unavailable())?.into_bytes();
                append_tail(&mut output, &bytes, &mut truncated);
            }
        }
        let text = String::from_utf8_lossy(&output);
        truncated |= text.len() > LOG_LIMIT;
        let mut start = text.len().saturating_sub(LOG_LIMIT);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        Ok::<_, ApiError>(
            json!({"logs":&text[start..],"truncated":truncated,"tail":input.tail,"limit_bytes":LOG_LIMIT,"container_id":app.active_container_id,"checked_at":Utc::now()}),
        )
    };
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .map_err(|_| unavailable())?
}
fn append_tail(output: &mut Vec<u8>, bytes: &[u8], truncated: &mut bool) {
    if output.len() + bytes.len() > LOG_LIMIT {
        *truncated = true;
        let discard = (output.len() + bytes.len() - LOG_LIMIT).min(output.len());
        output.drain(..discard);
    }
    output.extend_from_slice(&bytes[bytes.len().saturating_sub(LOG_LIMIT)..]);
}

#[derive(Clone, Serialize)]
struct Metrics {
    container_id: Option<String>,
    state: String,
    checked_at: DateTime<Utc>,
    cpu_cores_used: Option<f64>,
    cpu_limit_cores: Option<f64>,
    cpu_percent_of_limit: Option<f64>,
    memory_used_bytes: Option<u64>,
    memory_raw_bytes: Option<u64>,
    memory_limit_bytes: Option<u64>,
    memory_percent_of_limit: Option<f64>,
}
#[derive(Default)]
struct Cached {
    sample: Option<(Instant, Option<String>, Metrics)>,
}
type CacheSlots = HashMap<Uuid, (Instant, Arc<Mutex<Cached>>)>;
#[derive(Clone, Default)]
struct MetricsCache(Arc<Mutex<CacheSlots>>);
impl MetricsCache {
    async fn entry(&self, service: Uuid) -> Arc<Mutex<Cached>> {
        let mut slots = self.0.lock().await;
        if !slots.contains_key(&service)
            && slots.len() >= 256
            && let Some(oldest) = slots
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(id, _)| *id)
        {
            slots.remove(&oldest);
        }
        let slot = slots
            .entry(service)
            .or_insert_with(|| (Instant::now(), Arc::new(Mutex::new(Cached::default()))));
        slot.0 = Instant::now();
        slot.1.clone()
    }
}
async fn metrics(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Extension(cache): Extension<MetricsCache>,
) -> Result<Json<Metrics>, ApiError> {
    let app = app_access(&state, &id, service).await?;
    let read = async {
        let Some(container) = app.active_container_id.as_ref() else {
            return Ok(empty(None, "not_deployed", None));
        };
        // Check ownership before cache access. Cache entries never authorize a caller.
        let info = worker::inspect_owned(&state, app.id, container).await?;
        let host = info.host_config.as_ref();
        if !info.state.as_ref().is_some_and(|s| s.running == Some(true)) {
            let status = info
                .state
                .and_then(|s| s.status)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".into());
            return Ok(empty(Some(container.clone()), &status, host));
        }
        let generation = info
            .state
            .as_ref()
            .and_then(|s| s.started_at.as_ref())
            .map(ToString::to_string);
        let slot = cache.entry(service).await;
        let mut cached = slot.lock().await;
        if let Some((at, started, sample)) = &cached.sample
            && at.elapsed() < Duration::from_secs(3)
            && sample.container_id.as_ref() == Some(container)
            && started == &generation
        {
            return Ok(sample.clone());
        }
        let options = StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(false)
            .build();
        let stats = state
            .docker
            .stats(container, Some(options))
            .next()
            .await
            .ok_or_else(unavailable)?
            .map_err(|_| unavailable())?;
        let mut sample = empty(Some(container.clone()), "running", host);
        let cores = cpu_cores(&stats);
        let (raw, used) = memory_usage(&stats);
        sample.cpu_cores_used = cores;
        sample.cpu_percent_of_limit = cores
            .zip(sample.cpu_limit_cores)
            .map(|(usage, limit)| usage / limit * 100.0);
        sample.memory_raw_bytes = raw;
        sample.memory_used_bytes = used;
        sample.memory_percent_of_limit = used
            .zip(sample.memory_limit_bytes)
            .map(|(usage, limit)| usage as f64 / limit as f64 * 100.0);
        cached.sample = Some((Instant::now(), generation, sample.clone()));
        Ok(sample)
    };
    tokio::time::timeout(Duration::from_secs(7), read)
        .await
        .map_err(|_| unavailable())?
        .map(Json)
}
fn empty(container_id: Option<String>, state: &str, host: Option<&HostConfig>) -> Metrics {
    Metrics {
        container_id,
        state: state.into(),
        checked_at: Utc::now(),
        cpu_cores_used: None,
        cpu_limit_cores: host.and_then(cpu_limit),
        cpu_percent_of_limit: None,
        memory_used_bytes: None,
        memory_raw_bytes: None,
        memory_limit_bytes: host
            .and_then(|h| h.memory)
            .filter(|n| *n > 0)
            .map(|n| n as u64),
        memory_percent_of_limit: None,
    }
}
fn cpu_limit(host: &HostConfig) -> Option<f64> {
    let nano = host
        .nano_cpus
        .filter(|n| *n > 0)
        .map(|n| n as f64 / 1_000_000_000.0);
    let quota = host
        .cpu_quota
        .filter(|n| *n > 0)
        // Docker uses a 100 ms CFS period when no explicit period is stored.
        .map(|quota| quota as f64 / host.cpu_period.filter(|n| *n > 0).unwrap_or(100_000) as f64);
    let cpuset = host
        .cpuset_cpus
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(cpuset_count);
    [nano, quota, cpuset].into_iter().flatten().reduce(f64::min)
}
fn cpuset_count(value: &str) -> Option<f64> {
    let mut ranges = Vec::new();
    for part in value.split(',') {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let (first, last) = (first.parse::<u32>().ok()?, last.parse::<u32>().ok()?);
        if first > last {
            return None;
        }
        ranges.push((first, last));
    }
    ranges.sort_unstable();
    let mut end: Option<u32> = None;
    let mut count = 0u64;
    for (first, last) in ranges {
        let start = end
            .map(|end| u64::from(first).max(u64::from(end) + 1))
            .unwrap_or(u64::from(first));
        if start <= u64::from(last) {
            count += u64::from(last) - start + 1;
        }
        end = Some(end.unwrap_or(last).max(last));
    }
    (count > 0).then_some(count as f64)
}
fn memory_usage(stats: &ContainerStatsResponse) -> (Option<u64>, Option<u64>) {
    let memory = stats.memory_stats.as_ref();
    let raw = memory.and_then(|m| m.usage);
    let cache = memory
        .and_then(|m| m.stats.as_ref())
        .and_then(|m| {
            m.get("total_inactive_file")
                .or_else(|| m.get("inactive_file"))
                .or_else(|| m.get("cache"))
        })
        .copied()
        .unwrap_or(0);
    let used = raw.map(|usage| if cache < usage { usage - cache } else { usage });
    (raw, used)
}
fn cpu_cores(stats: &ContainerStatsResponse) -> Option<f64> {
    let current = stats.cpu_stats.as_ref()?;
    let previous = stats.precpu_stats.as_ref()?;
    let current_usage = current.cpu_usage.as_ref()?;
    let usage = current_usage
        .total_usage?
        .checked_sub(previous.cpu_usage.as_ref()?.total_usage?)?;
    if previous.system_cpu_usage? == 0 {
        return None;
    }
    let system = current
        .system_cpu_usage?
        .checked_sub(previous.system_cpu_usage?)?;
    if system == 0 {
        return None;
    }
    let cpus = current
        .online_cpus
        .filter(|n| *n > 0)
        .map(|n| n as usize)
        .or_else(|| current_usage.percpu_usage.as_ref().map(Vec::len))?;
    (cpus > 0).then_some(usage as f64 / system as f64 * cpus as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_buffer_keeps_latest_bytes_and_marks_overflow() {
        let mut bytes = vec![b'a'; LOG_LIMIT];
        let mut truncated = false;
        append_tail(&mut bytes, b"latest", &mut truncated);
        assert_eq!(bytes.len(), LOG_LIMIT);
        assert!(bytes.ends_with(b"latest"));
        assert!(truncated);
        append_tail(&mut bytes, &vec![b'z'; LOG_LIMIT + 100], &mut truncated);
        assert_eq!(bytes, vec![b'z'; LOG_LIMIT]);
    }
    #[test]
    fn cpu_uses_counter_deltas_and_real_service_capacity() {
        let mut stats: ContainerStatsResponse = serde_json::from_value(json!({"cpu_stats":{"cpu_usage":{"total_usage":200},"system_cpu_usage":1000,"online_cpus":4},"precpu_stats":{"cpu_usage":{"total_usage":100},"system_cpu_usage":600}})).unwrap();
        assert_eq!(cpu_cores(&stats), Some(1.0));
        stats.precpu_stats.as_mut().unwrap().system_cpu_usage = Some(1000);
        assert_eq!(cpu_cores(&stats), None);
        stats.precpu_stats.as_mut().unwrap().system_cpu_usage = Some(1100);
        assert_eq!(cpu_cores(&stats), None);
        let host = HostConfig {
            nano_cpus: Some(2_000_000_000),
            cpu_quota: Some(50000),
            cpu_period: Some(100000),
            ..Default::default()
        };
        assert_eq!(cpu_limit(&host), Some(0.5));
        assert_eq!(
            cpu_limit(&HostConfig {
                cpu_quota: Some(50000),
                ..Default::default()
            }),
            Some(0.5)
        );
        assert_eq!(cpu_limit(&HostConfig::default()), None);
        assert_eq!(cpuset_count("0,2-4,3-5"), Some(5.0));
        assert_eq!(cpuset_count("4-2"), None);
    }
    #[test]
    fn memory_excludes_inactive_cache_without_underflow_or_inventing_readings() {
        for key in ["total_inactive_file", "inactive_file", "cache"] {
            let mut values = serde_json::Map::new();
            values.insert(key.into(), json!(20));
            let stats: ContainerStatsResponse =
                serde_json::from_value(json!({"memory_stats":{"usage":100,"stats":values}}))
                    .unwrap();
            assert_eq!(memory_usage(&stats), (Some(100), Some(80)));
        }
        let bad_cache = serde_json::from_value(
            json!({"memory_stats":{"usage":100,"stats":{"inactive_file":200}}}),
        )
        .unwrap();
        assert_eq!(memory_usage(&bad_cache), (Some(100), Some(100)));
        assert_eq!(
            memory_usage(&ContainerStatsResponse::default()),
            (None, None)
        );
    }
}

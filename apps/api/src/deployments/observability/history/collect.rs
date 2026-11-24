use super::*;
use crate::notifications::{AlertPolicy, record};
fn prefix(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
pub(crate) async fn logs(
    state: &AppState,
    service: Uuid,
    deployment: Option<Uuid>,
    container: &str,
) -> Result<bool, ApiError> {
    worker::inspect_owned(state, service, container).await?;
    let last: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(emitted_at) FROM service_log_history WHERE service_id=$1 AND container_id=$2",
    )
    .bind(service)
    .bind(container)
    .fetch_one(&state.pool)
    .await?;
    let since = last
        .unwrap_or_else(|| Utc::now() - chrono::Duration::seconds(60))
        .timestamp()
        .clamp(0, i32::MAX as i64) as i32;
    let options = LogsOptionsBuilder::default()
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .since(since)
        .tail("5000")
        .build();
    let read = async {
        let mut stream = state.docker.logs(container, Some(options));
        let mut bytes = 0;
        let mut lines = 0;
        let mut truncated = false;
        let mut tx = state.pool.begin().await?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| unavailable())?.into_bytes();
            bytes += chunk.len();
            if bytes > 256 * 1024 {
                truncated = true;
                break;
            }
            let raw = String::from_utf8_lossy(&chunk);
            lines += raw.lines().count();
            truncated |= lines >= 5000;
            let (time, message) = raw
                .split_once(' ')
                .map(|(t, m)| {
                    (
                        DateTime::parse_from_rfc3339(t)
                            .ok()
                            .map(|d| d.with_timezone(&Utc)),
                        m,
                    )
                })
                .unwrap_or((None, &raw));
            let fingerprint = hex::encode(Sha256::digest(chunk.as_ref()));
            sqlx::query("INSERT INTO service_log_history(service_id,deployment_id,container_id,emitted_at,message,fingerprint,truncated) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(service_id,container_id,fingerprint) DO NOTHING").bind(service).bind(deployment).bind(container).bind(time).bind(prefix(message,16384)).bind(fingerprint).bind(message.len()>16384).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok::<_, ApiError>(truncated)
    };
    tokio::time::timeout(Duration::from_secs(6), read)
        .await
        .map_err(|_| unavailable())?
}
pub(super) async fn capture(
    state: &AppState,
    app: Application,
    config: &Settings,
    cache: &MetricsCache,
) -> Result<(), ApiError> {
    let measured = sample(state, &app, cache).await;
    let mut data = measured
        .as_ref()
        .ok()
        .and_then(|m| serde_json::to_value(m).ok())
        .unwrap_or_else(|| json!({"state":"unavailable"}));
    let mut image = None;
    let mut inspect_failed = false;
    if let Some(container) = &app.active_container_id {
        match worker::inspect_owned(state, app.id, container).await {
            Ok(info) => {
                image = info.image;
                data["restart_count"] = json!(info.restart_count);
                data["health"] = json!(
                    info.state
                        .as_ref()
                        .and_then(|s| s.health.as_ref())
                        .and_then(|h| h.status.as_ref())
                        .map(ToString::to_string)
                );
                data["started_at"] = json!(info.state.and_then(|s| s.started_at));
            }
            Err(_) => inspect_failed = true,
        }
    }
    let previous:Option<(Option<String>,Value)>=sqlx::query_as("SELECT container_id,data FROM telemetry_samples WHERE service_id=$1 ORDER BY collected_at DESC,id DESC LIMIT 1").bind(app.id).fetch_optional(&state.pool).await?;
    let marker = previous.as_ref().is_none_or(|(container, old)| {
        container != &app.active_container_id
            || old["restart_count"] != data["restart_count"]
            || old["health"] != data["health"]
            || old["started_at"] != data["started_at"]
            || old["state"] != data["state"]
    });
    let mut log_gap = false;
    if let Some(container) = &app.active_container_id {
        log_gap = logs(state, app.id, app.active_deployment_id, container)
            .await
            .unwrap_or(true);
    }
    data["log_gap"] = json!(log_gap);
    sqlx::query("INSERT INTO telemetry_samples(service_id,deployment_id,container_id,image_id,data,error,marker) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(app.id).bind(app.active_deployment_id).bind(&app.active_container_id).bind(image).bind(&data).bind(measured.is_err()||inspect_failed).bind(marker).execute(&state.pool).await?;
    let policy = |threshold| AlertPolicy {
        threshold,
        sustained_seconds: config.sustained_seconds,
        gap_seconds: (config.sample_seconds * 4).max(120),
    };
    record(
        state,
        app.id,
        "cpu",
        app.active_container_id.as_deref(),
        measured.as_ref().ok().and_then(|m| m.cpu_percent_of_limit),
        policy(config.cpu_threshold),
    )
    .await?;
    record(
        state,
        app.id,
        "memory",
        app.active_container_id.as_deref(),
        measured
            .as_ref()
            .ok()
            .and_then(|m| m.memory_percent_of_limit),
        policy(config.memory_threshold),
    )
    .await?;
    let restarts:i64=sqlx::query_scalar("SELECT COALESCE(max((data->>'restart_count')::bigint)-min((data->>'restart_count')::bigint),0) FROM telemetry_samples WHERE service_id=$1 AND container_id=$2 AND collected_at>now()-make_interval(mins=>$3::int) AND data->>'restart_count' IS NOT NULL").bind(app.id).bind(&app.active_container_id).bind(config.restart_window_minutes as i32).fetch_one(&state.pool).await?;
    alert(
        state,
        Some(app.id),
        "restarts",
        Some(restarts >= config.restart_threshold),
        0,
        config,
    )
    .await?;
    Ok(())
}
async fn alert(
    state: &AppState,
    service: Option<Uuid>,
    resource: &str,
    high: Option<bool>,
    sustain: i64,
    config: &Settings,
) -> Result<(), ApiError> {
    let Some(high) = high else { return Ok(()) };
    let key = service.unwrap_or(Uuid::nil());
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO telemetry_alerts(scope_id,resource) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(key)
    .bind(resource)
    .execute(&mut *tx)
    .await?;
    let (active,since,last):(bool,Option<DateTime<Utc>>,Option<DateTime<Utc>>)=sqlx::query_as("SELECT active,high_since,last_sample FROM telemetry_alerts WHERE scope_id=$1 AND resource=$2 FOR UPDATE").bind(key).bind(resource).fetch_one(&mut *tx).await?;
    let now = Utc::now();
    let since = if high {
        Some(
            if last.is_none_or(|l| {
                now - l > chrono::Duration::seconds((config.sample_seconds * 4).max(120))
            }) {
                now
            } else {
                since.unwrap_or(now)
            },
        )
    } else {
        None
    };
    let next = if high {
        active || since.is_some_and(|s| now - s >= chrono::Duration::seconds(sustain))
    } else {
        false
    };
    sqlx::query("UPDATE telemetry_alerts SET active=$3,high_since=$4,last_sample=$5 WHERE scope_id=$1 AND resource=$2").bind(key).bind(resource).bind(next).bind(since).bind(now).execute(&mut *tx).await?;
    if active != next {
        let projects: Vec<(Uuid, Uuid)> = if let Some(service) = service {
            sqlx::query_as("SELECT p.id,p.organization_id FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1").bind(service).fetch_all(&mut *tx).await?
        } else {
            sqlx::query_as("SELECT id,organization_id FROM projects")
                .fetch_all(&mut *tx)
                .await?
        };
        for (project, org) in projects {
            sqlx::query("INSERT INTO notifications(id,organization_id,project_id,service_id,kind,title,message) VALUES($1,$2,$3,$4,'telemetry_alert',$5,$6)").bind(Uuid::new_v4()).bind(org).bind(project).bind(service).bind(match(resource,next){("restarts",true)=>"Repeated service restarts",("restarts",false)=>"Service restart rate recovered",(_,true)=>"Host disk pressure",(_,false)=>"Host disk usage recovered"}).bind(if next{"The configured sustained resource/restart threshold was exceeded. Review Monitoring history."}else{"The monitored resource is below its configured alert threshold."}).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
async fn host(state: &AppState, config: &Settings) -> Result<(), ApiError> {
    let data = crate::monitoring::latest().filter(|v| {
        v["at"]
            .as_i64()
            .is_some_and(|t| Utc::now().timestamp_millis() - t < 30_000)
    });
    let error = data.is_none();
    let data = data.unwrap_or_else(|| json!({"state":"unavailable"}));
    let disk = data["disk_used"]
        .as_f64()
        .zip(data["disk_total"].as_f64())
        .filter(|(_, total)| *total > 0.)
        .map(|(used, total)| used / total * 100. >= config.disk_threshold);
    sqlx::query("INSERT INTO telemetry_samples(data,error) VALUES($1,$2)")
        .bind(data)
        .bind(error)
        .execute(&state.pool)
        .await?;
    alert(state, None, "disk", disk, config.sustained_seconds, config).await
}
pub(crate) async fn run_worker(state: AppState, mut stop: tokio::sync::watch::Receiver<bool>) {
    let cache = MetricsCache::default();
    let mut cursor = Uuid::nil();
    loop {
        let config = match settings(&state).await {
            Ok(v) => v,
            Err(_) => {
                tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(30))=>{}}
                continue;
            }
        };
        let rows = sqlx::query_as::<_, Application>(&format!(
            "SELECT {APP_COLUMNS} FROM applications a WHERE a.id>$1 ORDER BY a.id LIMIT 32"
        ))
        .bind(cursor)
        .fetch_all(&state.pool)
        .await;
        if let Ok(rows) = rows {
            cursor = if rows.len() < 32 {
                Uuid::nil()
            } else {
                rows.last().unwrap().id
            };
            let collection = stream::iter(rows).for_each_concurrent(4, |app| {
                let state = &state;
                let config = &config;
                let cache = &cache;
                async move {
                    if capture(state, app, config, cache).await.is_err() {
                        tracing::debug!("Service history collection unavailable");
                    }
                }
            });
            tokio::select! {_=stop.changed()=>break,_=collection=>{}}
        }
        if host(&state, &config).await.is_err() {
            tracing::debug!("Host history collection unavailable");
        }
        if retention::prune(&state, &config).await.is_err() {
            tracing::warn!("Telemetry retention pass failed");
        }
        tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(config.sample_seconds as u64))=>{}}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_log_truncation_preserves_valid_text() {
        assert_eq!(prefix("ééé", 5), "éé");
    }
}

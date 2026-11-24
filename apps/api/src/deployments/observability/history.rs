//! Bounded background telemetry; gaps stay unknown and logs never authorize access.
use super::*;
use futures_util::stream;
use sha2::{Digest, Sha256};
#[derive(Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(deny_unknown_fields)]
struct Settings {
    sample_seconds: i64,
    retention_days: i64,
    log_retention_days: i64,
    max_storage_mib: i64,
    downsample_after_minutes: i64,
    downsample_seconds: i64,
    cpu_threshold: f64,
    memory_threshold: f64,
    disk_threshold: f64,
    sustained_seconds: i64,
    restart_threshold: i64,
    restart_window_minutes: i64,
}
async fn settings(state: &AppState) -> Result<Settings, ApiError> {
    sqlx::query_as("SELECT * FROM telemetry_settings WHERE singleton")
        .fetch_one(&state.pool)
        .await
        .map_err(Into::into)
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/history", get(history))
        .route("/services/{id}/logs/history", get(log_history))
        .route("/monitoring/history", get(host_history))
        .route(
            "/monitoring/settings",
            get(get_settings).post(save_settings),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
#[derive(Deserialize)]
struct Range {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    before: Option<i64>,
}
impl Range {
    fn validate(&self) -> Result<(), ApiError> {
        if self.from >= self.to
            || self.to - self.from > chrono::Duration::days(31)
            || self.q.len() > 200
        {
            return Err(invalid(
                "Choose a range of up to 31 days and a search of up to 200 characters.",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, sqlx::FromRow)]
struct Point {
    id: i64,
    service_id: Option<Uuid>,
    deployment_id: Option<Uuid>,
    container_id: Option<String>,
    image_id: Option<String>,
    collected_at: DateTime<Utc>,
    data: Value,
    error: bool,
    marker: bool,
}
async fn points(
    state: &AppState,
    service: Option<Uuid>,
    r: Range,
) -> Result<Json<Value>, ApiError> {
    r.validate()?;
    let mut rows:Vec<Point>=sqlx::query_as("SELECT * FROM telemetry_samples WHERE service_id IS NOT DISTINCT FROM $1 AND collected_at>=$2 AND collected_at<=$3 ORDER BY collected_at DESC,id DESC LIMIT 1001").bind(service).bind(r.from).bind(r.to).fetch_all(&state.pool).await?;
    let truncated = rows.len() > 1000;
    rows.truncate(1000);
    rows.reverse();
    let config = settings(state).await?;
    Ok(Json(
        json!({"samples":rows,"truncated":truncated,"sample_seconds":config.sample_seconds,"retention_days":config.retention_days,"downsample_after_minutes":config.downsample_after_minutes,"downsample_seconds":config.downsample_seconds,"from":r.from,"to":r.to,"scope":if service.is_some(){"service"}else{"API host OS namespace; not physical-host or Docker VM capacity"},"gaps_note":"Missing/error samples and intervals between recorded points are unknown, not zero. Older data retains boundary/peak samples; a truncated result covers only the newest 1000 points."}),
    ))
}
async fn history(
    State(s): State<AppState>,
    id: Identity,
    Path(a): Path<Uuid>,
    Query(r): Query<Range>,
) -> Result<Json<Value>, ApiError> {
    app_access(&s, &id, a).await?;
    points(&s, Some(a), r).await
}
async fn host_history(
    State(s): State<AppState>,
    id: Identity,
    Query(r): Query<Range>,
) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    points(&s, None, r).await
}
#[derive(Serialize, sqlx::FromRow)]
struct Line {
    id: i64,
    deployment_id: Option<Uuid>,
    container_id: String,
    emitted_at: Option<DateTime<Utc>>,
    collected_at: DateTime<Utc>,
    message: String,
    truncated: bool,
}
async fn log_history(
    State(s): State<AppState>,
    id: Identity,
    Path(a): Path<Uuid>,
    Query(r): Query<Range>,
) -> Result<Json<Value>, ApiError> {
    app_access(&s, &id, a).await?;
    r.validate()?;
    let mut rows:Vec<Line>=sqlx::query_as("SELECT id,deployment_id,container_id,emitted_at,collected_at,message,truncated FROM service_log_history WHERE service_id=$1 AND collected_at>=$2 AND collected_at<=$3 AND ($4::bigint IS NULL OR id<$4) AND ($5='' OR position(lower($5) in lower(message))>0) ORDER BY id DESC LIMIT 201").bind(a).bind(r.from).bind(r.to).bind(r.before).bind(&r.q).fetch_all(&s.pool).await?;
    let more = rows.len() > 200;
    rows.truncate(200);
    let next = rows.last().map(|l| l.id);
    rows.reverse();
    Ok(Json(
        json!({"lines":rows,"next_before":if more{next}else{None},"collection_note":"emitted_at is Docker's log timestamp; collected_at is when Ferrix retained it. Embedded application timestamps are preserved in the message. Retention, Docker rotation and bounded collection can leave gaps."}),
    ))
}
async fn get_settings(State(s): State<AppState>, id: Identity) -> Result<Json<Settings>, ApiError> {
    id.admin()?;
    settings(&s).await.map(Json)
}
async fn save_settings(
    State(s): State<AppState>,
    id: Identity,
    Json(v): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    if !(10..=300).contains(&v.sample_seconds)
        || !(1..=30).contains(&v.retention_days)
        || !(1..=30).contains(&v.log_retention_days)
        || !(16..=1024).contains(&v.max_storage_mib)
        || !(60..=10080).contains(&v.downsample_after_minutes)
        || !(60..=3600).contains(&v.downsample_seconds)
        || [v.cpu_threshold, v.memory_threshold, v.disk_threshold]
            .iter()
            .any(|t| !t.is_finite() || !(10.0..=100.).contains(t))
        || !(30..=3600).contains(&v.sustained_seconds)
        || !(1..=100).contains(&v.restart_threshold)
        || !(1..=60).contains(&v.restart_window_minutes)
    {
        return Err(invalid(
            "Telemetry settings are outside their supported ranges.",
        ));
    }
    sqlx::query("UPDATE telemetry_settings SET sample_seconds=$1,retention_days=$2,log_retention_days=$3,max_storage_mib=$4,downsample_after_minutes=$5,downsample_seconds=$6,cpu_threshold=$7,memory_threshold=$8,disk_threshold=$9,sustained_seconds=$10,restart_threshold=$11,restart_window_minutes=$12 WHERE singleton").bind(v.sample_seconds).bind(v.retention_days).bind(v.log_retention_days).bind(v.max_storage_mib).bind(v.downsample_after_minutes).bind(v.downsample_seconds).bind(v.cpu_threshold).bind(v.memory_threshold).bind(v.disk_threshold).bind(v.sustained_seconds).bind(v.restart_threshold).bind(v.restart_window_minutes).execute(&s.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
mod collect;
mod retention;
pub(crate) use collect::run_worker;

pub(crate) use collect::logs as capture_retired_logs;

#[cfg(test)]
mod tests;

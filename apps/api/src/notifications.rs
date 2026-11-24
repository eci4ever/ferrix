//! Organization inbox; events contain safe labels, never logs or credentials.
use crate::{ApiError, AppState, auth::Identity};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/notifications", get(list))
        .route("/api/v1/notifications/read-all", post(read_all))
        .route("/api/v1/notifications/{id}/read", post(read))
}
#[derive(Serialize, FromRow)]
struct Notification {
    id: Uuid,
    project_id: Uuid,
    service_id: Option<Uuid>,
    kind: String,
    title: String,
    message: String,
    created_at: DateTime<Utc>,
    read: bool,
}
async fn list(State(state): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    let rows:Vec<Notification>=sqlx::query_as("SELECT n.id,n.project_id,n.service_id,n.kind,n.title,n.message,n.created_at,EXISTS(SELECT 1 FROM notification_reads r WHERE r.notification_id=n.id AND r.user_id=$2) AS read FROM notifications n WHERE n.organization_id=$1 AND n.expires_at>now() ORDER BY n.created_at DESC,n.id DESC LIMIT 100")
 .bind(id.organization_id).bind(id.user_id).fetch_all(&state.pool).await?;
    let unread:i64=sqlx::query_scalar("SELECT count(*) FROM notifications n WHERE n.organization_id=$1 AND n.expires_at>now() AND NOT EXISTS(SELECT 1 FROM notification_reads r WHERE r.notification_id=n.id AND r.user_id=$2)").bind(id.organization_id).bind(id.user_id).fetch_one(&state.pool).await?;
    Ok(Json(json!({"notifications":rows,"unread":unread})))
}
async fn read(
    State(state): State<AppState>,
    id: Identity,
    Path(notification): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM notifications WHERE id=$1 AND organization_id=$2 AND expires_at>now())").bind(notification).bind(id.organization_id).fetch_one(&state.pool).await?;
    if !exists {
        return Err(ApiError(
            axum::http::StatusCode::NOT_FOUND,
            "Notification not found.",
        ));
    }
    sqlx::query("INSERT INTO notification_reads(notification_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(notification).bind(id.user_id).execute(&state.pool).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
async fn read_all(
    State(state): State<AppState>,
    id: Identity,
) -> Result<axum::http::StatusCode, ApiError> {
    sqlx::query("INSERT INTO notification_reads(notification_id,user_id) SELECT id,$2 FROM notifications WHERE organization_id=$1 AND expires_at>now() ON CONFLICT DO NOTHING").bind(id.organization_id).bind(id.user_id).execute(&state.pool).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
// Hysteresis: sustained >=90% for 60 seconds; re-arm only below 80%. Missing
// samples or a gap >120s reset the window instead of inventing continuous load.
#[derive(Clone, Copy, Default)]
struct AlertState {
    since: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    notified: bool,
}
#[cfg(test)]
fn observe(state: AlertState, usage: Option<f64>, now: DateTime<Utc>) -> (AlertState, bool) {
    observe_policy(
        state,
        usage,
        now,
        AlertPolicy {
            threshold: 90.,
            sustained_seconds: 60,
            gap_seconds: 120,
        },
    )
}
#[derive(Clone, Copy)]
pub(crate) struct AlertPolicy {
    pub threshold: f64,
    pub sustained_seconds: i64,
    pub gap_seconds: i64,
}
fn observe_policy(
    mut state: AlertState,
    usage: Option<f64>,
    now: DateTime<Utc>,
    policy: AlertPolicy,
) -> (AlertState, bool) {
    if state
        .last
        .is_none_or(|last| now - last > chrono::Duration::seconds(policy.gap_seconds))
    {
        state = AlertState::default()
    }
    state.last = Some(now);
    let Some(usage) = usage.filter(|v| v.is_finite()) else {
        return (
            AlertState {
                last: Some(now),
                ..Default::default()
            },
            false,
        );
    };
    if usage < (policy.threshold - 10.).max(0.) {
        return (
            AlertState {
                last: Some(now),
                ..Default::default()
            },
            false,
        );
    }
    if usage < policy.threshold {
        state.since = None;
        return (state, false);
    }
    let since = *state.since.get_or_insert(now);
    let emit =
        !state.notified && now - since >= chrono::Duration::seconds(policy.sustained_seconds);
    if emit {
        state.notified = true
    }
    (state, emit)
}
pub(crate) async fn record(
    state: &AppState,
    service: Uuid,
    resource: &str,
    container: Option<&str>,
    usage: Option<f64>,
    policy: AlertPolicy,
) -> Result<(), ApiError> {
    let now = Utc::now();
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO service_resource_alerts(service_id,resource,last_sample) SELECT id,$2,$3 FROM applications WHERE id=$1 ON CONFLICT DO NOTHING").bind(service).bind(resource).bind(now).execute(&mut *tx).await?;
    type AlertRow = (Option<DateTime<Utc>>, DateTime<Utc>, bool, Option<String>);
    let row:Option<AlertRow>=sqlx::query_as("SELECT high_since,last_sample,notified,container_id FROM service_resource_alerts WHERE service_id=$1 AND resource=$2 FOR UPDATE").bind(service).bind(resource).fetch_optional(&mut *tx).await?;
    let Some((since, last, notified, previous)) = row else {
        return Ok(());
    };
    let (next, emit) = observe_policy(
        if previous.as_deref() == container {
            AlertState {
                since,
                last: Some(last),
                notified,
            }
        } else {
            AlertState::default()
        },
        usage,
        now,
        policy,
    );
    sqlx::query("UPDATE service_resource_alerts SET high_since=$3,last_sample=$4,notified=$5,container_id=$6 WHERE service_id=$1 AND resource=$2").bind(service).bind(resource).bind(next.since).bind(now).bind(next.notified).bind(container).execute(&mut *tx).await?;
    if emit {
        sqlx::query("INSERT INTO notifications(id,organization_id,project_id,service_id,kind,title,message) SELECT $1,p.organization_id,p.id,a.id,'resource_high',$3,a.name || ' · ' || p.name || ' / ' || e.name FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$2").bind(Uuid::new_v4()).bind(service).bind(if resource=="cpu"{"High CPU usage"}else{"High memory usage"}).execute(&mut *tx).await?;
    }
    if notified
        && !next.notified
        && usage.is_some_and(|v| v < (policy.threshold - 10.).max(0.))
        && previous.as_deref() == container
    {
        sqlx::query("INSERT INTO notifications(id,organization_id,project_id,service_id,kind,title,message) SELECT $1,p.organization_id,p.id,a.id,'resource_recovered',$3,a.name || ' · resource usage returned below its recovery threshold' FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$2").bind(Uuid::new_v4()).bind(service).bind(if resource=="cpu"{"CPU usage recovered"}else{"Memory usage recovered"}).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn run_worker(state: AppState, stop: tokio::sync::watch::Receiver<bool>) {
    crate::deployments::observability::history::run_worker(state, stop).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sustained_load_hysteresis_and_missing_samples() {
        let t = Utc::now();
        let (s, emit) = observe(AlertState::default(), Some(95.), t);
        assert!(!emit);
        let (s, emit) = observe(s, Some(95.), t + chrono::Duration::seconds(60));
        assert!(emit);
        let (s, emit) = observe(s, Some(99.), t + chrono::Duration::seconds(90));
        assert!(!emit);
        let (s, _) = observe(s, Some(79.), t + chrono::Duration::seconds(100));
        assert!(!s.notified);
        let (s, _) = observe(s, Some(95.), t + chrono::Duration::seconds(110));
        let (s, emit) = observe(s, None, t + chrono::Duration::seconds(120));
        assert!(!emit);
        assert!(s.since.is_none());
        let (s, _) = observe(s, Some(99.), t + chrono::Duration::seconds(130));
        assert!(!observe(s, Some(99.), t + chrono::Duration::seconds(300)).1);
    }
}

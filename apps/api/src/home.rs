use crate::{ApiError, AppState, auth::Identity};
use axum::{Json, extract::State};
use serde_json::Value;

// One statement gives every metric the same database snapshot. No list limits
// or Docker host inventory are used to infer organization totals/runtime health.
const SUMMARY: &str = r#"
WITH org_projects AS MATERIALIZED (
    SELECT id, created_at FROM projects WHERE organization_id = $1 AND ($2::UUID IS NULL OR id=$2) AND ferrix_project_visible($3,$1,id)
), org_services AS MATERIALIZED (
    SELECT a.id, a.name, a.service_kind, e.project_id
    FROM applications a JOIN environments e ON e.id = a.environment_id
    JOIN org_projects p ON p.id = e.project_id
    WHERE a.service_kind IN ('application','database') AND ferrix_service_permission($3,$1,a.id) IS NOT NULL
), recent_deployments AS MATERIALIZED (
    SELECT d.status FROM deployments d JOIN org_services s ON s.id = d.application_id
    WHERE d.created_at >= now() - interval '24 hours'
), attention AS MATERIALIZED (
    SELECT s.id, s.name, s.project_id, d.created_at
    FROM org_services s
    JOIN LATERAL (
        SELECT status, created_at FROM deployments WHERE application_id = s.id
        ORDER BY created_at DESC, id DESC LIMIT 1
    ) d ON d.status = 'failed'
)
SELECT json_build_object(
    'projects', (SELECT count(*) FROM org_projects),
    'projects_new_7d', (SELECT count(*) FROM org_projects WHERE created_at >= now() - interval '7 days'),
    'services', (SELECT count(*) FROM org_services),
    'applications', (SELECT count(*) FROM org_services WHERE service_kind = 'application'),
    'databases', (SELECT count(*) FROM org_services WHERE service_kind = 'database'),
    'deployments_24h', (SELECT count(*) FROM recent_deployments),
    'succeeded_24h', (SELECT count(*) FROM recent_deployments WHERE status = 'succeeded'),
    'failed_24h', (SELECT count(*) FROM recent_deployments WHERE status = 'failed'),
    'in_progress_24h', (SELECT count(*) FROM recent_deployments WHERE status IN ('queued', 'running')),
    'needs_attention', (SELECT count(*) FROM attention),
    'attention', COALESCE((SELECT json_agg(row_to_json(item)) FROM (
        SELECT id, name, project_id FROM attention ORDER BY created_at DESC, id DESC LIMIT 5
    ) item), '[]'::json)
)
"#;

pub(crate) async fn get(
    State(state): State<AppState>,
    id: Identity,
    key: Option<axum::Extension<crate::auth::api_keys::KeyAccess>>,
) -> Result<Json<Value>, ApiError> {
    let summary: sqlx::types::Json<Value> = sqlx::query_scalar(SUMMARY)
        .bind(id.organization_id)
        .bind(key.and_then(|key| key.project_id))
        .bind(id.user_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(summary.0))
}

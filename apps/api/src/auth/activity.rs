//! Activity reads are scoped in SQL. Request events store route templates, never bodies or queries.
use super::*;
use axum::{
    Json,
    extract::{MatchedPath, Query, Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Default, Deserialize)]
pub(super) struct Search {
    #[serde(default)]
    q: String,
    organization_id: Option<Uuid>,
    user_id: Option<Uuid>,
    #[serde(default)]
    page: u32,
}
pub(super) async fn list(
    State(state): State<AppState>,
    id: Identity,
    Query(query): Query<Search>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    if query.q.chars().count() > 120 || query.page > 100_000 {
        return Err(bad("Search is limited to 120 characters and 100000 pages."));
    }
    let platform = id.system_role == "admin";
    if !platform
        && query
            .organization_id
            .is_some_and(|org| org != id.organization_id)
    {
        return Err(forbidden());
    }
    let manager = matches!(id.role.as_str(), "owner" | "admin");
    if !platform && !manager && query.user_id.is_some_and(|user| user != id.user_id) {
        return Err(forbidden());
    }
    let org = if platform {
        query.organization_id
    } else {
        Some(id.organization_id)
    };
    let user = if !platform && !manager {
        Some(id.user_id)
    } else {
        query.user_id
    };
    // One statement gives count and page the same snapshot, including empty/out-of-range pages.
    let result: Value=sqlx::query_scalar(r#"WITH visible AS (
      SELECT id,actor_id AS user_id,COALESCE(actor_name,'Deleted or unknown user') AS user_name,
      organization_ref AS organization_id,COALESCE(organization_name,'Deleted or unknown organization') AS organization_name,
      action,outcome,target_id,created_at FROM auth_audit
      WHERE ($3 OR organization_ref=$6) AND ($3 OR $7 OR actor_id=$8)
      AND ($3 OR (action NOT LIKE 'admin.%' AND action NOT LIKE 'platform.%'))
    ), scoped AS (SELECT * FROM visible WHERE ($1::uuid IS NULL OR organization_id=$1) AND ($2::uuid IS NULL OR user_id=$2)
      AND ($4='' OR position(lower($4) in lower(action||' '||COALESCE(user_name,'')||' '||COALESCE(organization_name,'')||' '||COALESCE(target_id::text,'')))>0)
    ), paged AS (SELECT * FROM scoped ORDER BY created_at DESC,id DESC LIMIT 25 OFFSET $5)
    SELECT json_build_object('items',COALESCE((SELECT json_agg(paged ORDER BY created_at DESC,id DESC) FROM paged),'[]'::json),'total',(SELECT count(*) FROM scoped),'users',COALESCE((SELECT json_agg(v) FROM (SELECT DISTINCT ON(user_id) user_id AS id,user_name AS name FROM visible WHERE user_id IS NOT NULL ORDER BY user_id,created_at DESC,id DESC LIMIT 500) v),'[]'::json),'organizations',COALESCE((SELECT json_agg(v) FROM (SELECT DISTINCT ON(organization_id) organization_id AS id,organization_name AS name FROM visible WHERE organization_id IS NOT NULL ORDER BY organization_id,created_at DESC,id DESC LIMIT 500) v),'[]'::json))::jsonb"#)
      .bind(org).bind(user).bind(platform).bind(query.q.trim()).bind(i64::from(query.page)*25).bind(id.organization_id).bind(manager).bind(id.user_id).fetch_one(&state.pool).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(result)))
}

pub(super) async fn run(state: &AppState, req: Request, next: Next) -> Response {
    use axum::response::IntoResponse;
    let write = !matches!(req.method().as_str(), "GET" | "HEAD" | "OPTIONS");
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let identity = req.extensions().get::<Identity>().cloned();
    // Auth already has transactional semantic events; avoid duplicating those here.
    let Some((id, route)) = identity
        .zip(route)
        .filter(|(_, route)| write && !route.contains("/auth/"))
    else {
        return next.run(req).await;
    };
    let event = Uuid::new_v4();
    let target = req
        .uri()
        .path()
        .split('/')
        .find_map(|part| Uuid::parse_str(part).ok());
    let prefix = if route.contains("/docker/swarm/")
        || route.contains("/releases/update")
        || route.contains("/monitoring/")
    {
        "platform.request"
    } else {
        "request"
    };
    let action = format!("{prefix}.{} {}", req.method(), route);
    // Fail closed before mutations. If interrupted later, pending explicitly means unknown outcome.
    if sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action,target_id,outcome) VALUES($1,$2,$3,$4,$5,'pending')")
        .bind(event).bind(id.user_id).bind(id.organization_id).bind(action).bind(target).execute(&state.pool).await.is_err() {
        return ApiError(StatusCode::SERVICE_UNAVAILABLE,"Activity could not be recorded. Try again later.").into_response();
    }
    let response = next.run(req).await;
    let outcome = if response.status().is_success() {
        "succeeded"
    } else {
        "failed"
    };
    if sqlx::query("UPDATE auth_audit SET outcome=$2 WHERE id=$1")
        .bind(event)
        .bind(outcome)
        .execute(&state.pool)
        .await
        .is_err()
    {
        tracing::error!(event_id=%event,"Activity outcome update failed; event remains pending");
    }
    response
}

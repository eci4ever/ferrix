pub mod auth;
mod projects;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use bollard::{Docker, query_parameters::ListContainersOptionsBuilder};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub docker: Docker,
    pub auth: std::sync::Arc<auth::AuthConfig>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/v1/status", get(status))
        .route(
            "/api/v1/projects",
            get(projects::list).post(projects::create),
        )
        .route("/api/v1/containers", get(containers))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::sessions::protect,
        ));
    Router::new()
        .merge(protected)
        .nest("/api/v1/auth", auth::routes(state.clone()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::sessions::guard,
        ))
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"service":"ferrix", "status":"ok"})) }),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize)]
struct SystemStatus {
    database: bool,
    docker: bool,
    version: &'static str,
}

async fn status(State(state): State<AppState>) -> Json<SystemStatus> {
    let (database, docker) = tokio::join!(
        sqlx::query("SELECT 1").execute(&state.pool),
        tokio::time::timeout(Duration::from_secs(3), state.docker.ping())
    );
    Json(SystemStatus {
        database: database.is_ok(),
        docker: matches!(docker, Ok(Ok(_))),
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
struct Container {
    id: String,
    name: String,
    image: String,
    state: String,
    status: String,
}

async fn containers(
    State(state): State<AppState>,
    id: auth::Identity,
) -> Result<Json<Vec<Container>>, ApiError> {
    id.admin()?;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.list_containers(Some(
            ListContainersOptionsBuilder::default().all(true).build(),
        )),
    )
    .await;
    let items = match result {
        Ok(Ok(items)) => items,
        failure => {
            tracing::warn!(?failure, "Docker inventory unavailable");
            return Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Docker is unavailable. Check the daemon and DOCKER_HOST.",
            ));
        }
    };
    Ok(Json(
        items
            .into_iter()
            .map(|item| Container {
                id: item.id.unwrap_or_default(),
                name: item
                    .names
                    .unwrap_or_default()
                    .first()
                    .map(|n| n.trim_start_matches('/').to_owned())
                    .unwrap_or_default(),
                image: item.image.unwrap_or_default(),
                state: item.state.map(|s| s.to_string()).unwrap_or_default(),
                status: item.status.unwrap_or_default(),
            })
            .collect(),
    ))
}

pub struct ApiError(StatusCode, &'static str);

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "Database request failed");
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database is unavailable. Try again shortly.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}

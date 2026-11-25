pub mod auth;
pub mod deployments;
mod docker_inventory;
pub mod gateway;
mod home;
pub mod housekeeping;
mod monitoring;
pub mod notifications;
mod projects;
mod releases;
pub mod self_update;
mod server_time;

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
    pub gateway: std::sync::Arc<gateway::GatewayConfig>,
    pub pool: PgPool,
    pub docker: Docker,
    pub auth: std::sync::Arc<auth::AuthConfig>,
}

pub fn router(state: AppState) -> Router {
    let monitor = monitoring::start();
    let release_cache = releases::start();
    let protected = Router::new()
        .merge(notifications::routes())
        .route("/api/v1/agent", get(auth::api_keys::agent))
        .route("/api/v1/agent/docs", get(auth::api_keys::documentation))
        .route("/api/v1/monitoring", get(monitoring::get))
        .route("/api/v1/docker/{section}", get(docker_inventory::get))
        .route(
            "/api/v1/docker/swarm/services/{id}/restart",
            axum::routing::post(docker_inventory::restart),
        )
        .route(
            "/api/v1/docker/swarm/services/{id}/scale",
            axum::routing::post(docker_inventory::scale),
        )
        .layer(axum::Extension(monitor))
        .route("/api/v1/home-summary", get(home::get))
        .route("/api/v1/status", get(status))
        .route(
            "/api/v1/projects",
            get(projects::list).post(projects::create),
        )
        .route("/api/v1/containers", get(containers))
        .route(
            "/api/v1/projects/{id}",
            axum::routing::delete(projects::remove),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::sessions::protect,
        ));
    let clock = Router::new()
        .route(
            "/api/v1/docker/swarm/services/{id}/logs",
            get(docker_inventory::logs),
        )
        .route("/api/v1/projects/service-status", get(projects::statuses))
        .route("/api/v1/server-time", get(server_time::get))
        .route("/api/v1/releases/latest", get(releases::get))
        .route(
            "/api/v1/releases/update",
            get(self_update::status).post(self_update::start),
        )
        .layer(axum::Extension(release_cache))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::sessions::identify,
        ));
    Router::new()
        .nest("/api/v1", auth::permissions::routes(state.clone()))
        .nest("/api/v1", deployments::routes(state.clone()))
        .merge(clock)
        .merge(protected)
        .nest("/api/v1/auth", auth::routes(state.clone()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::sessions::guard,
        ))
        .merge(deployments::webhook_routes(state.clone()))
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"service":"ferrix", "status":"ok"})) }),
        )
        .layer(axum::middleware::from_fn(self_update::guard))
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

#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str);

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        if error
            .as_database_error()
            .is_some_and(|e| e.constraint() == Some("ferrix_project_maintenance"))
        {
            return Self(
                StatusCode::CONFLICT,
                "Project maintenance is active. Wait for coordinated backup/restore to finish.",
            );
        }
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

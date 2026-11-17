use ferrix_api::{AppState, router};
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, time::Duration};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferrix_api=info,tower_http=info".into()),
        )
        .init();
    let bind: SocketAddr = std::env::var("FERRIX_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    let auth = std::sync::Arc::new(ferrix_api::auth::AuthConfig::from_env()?);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    sqlx::migrate!().run(&pool).await?;
    let docker = bollard::Docker::connect_with_defaults()?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(address = %listener.local_addr()?, "Ferrix API ready");
    let mut app = router(AppState {
        pool: pool.clone(),
        docker,
        auth,
    });
    if let Ok(directory) = std::env::var("FERRIX_STATIC_DIR") {
        let index = std::path::Path::new(&directory).join("index.html");
        if !index.is_file() {
            return Err("FERRIX_STATIC_DIR must contain index.html".into());
        }
        app = app
            .route(
                "/api",
                axum::routing::any(|| async { axum::http::StatusCode::NOT_FOUND }),
            )
            .route(
                "/api/{*unmatched}",
                axum::routing::any(|| async { axum::http::StatusCode::NOT_FOUND }),
            )
            .fallback_service(
                tower_http::services::ServeDir::new(directory)
                    .fallback(tower_http::services::ServeFile::new(index)),
            );
    }
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    pool.close().await;
    Ok(())
}

async fn shutdown() {
    let interrupt = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = interrupt => {}, _ = terminate => {} }
}

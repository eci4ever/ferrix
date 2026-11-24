use ferrix_api::{AppState, router};
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, time::Duration};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();
    match std::env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--schema-version") => {
            println!("{}", MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0));
            return Ok(());
        }
        Some(_) => return Err("Unknown argument".into()),
        None => {}
    }
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
    MIGRATOR.run(&pool).await?;
    let docker = bollard::Docker::connect_with_defaults()?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(address = %listener.local_addr()?, "Ferrix API ready");
    let state = AppState {
        gateway: std::sync::Arc::new(ferrix_api::gateway::GatewayConfig::from_env()?),
        pool: pool.clone(),
        docker,
        auth,
    };
    ferrix_api::self_update::initialize(&state).await;
    let mut app = router(state.clone());
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
    let (worker_shutdown, receiver) = tokio::sync::watch::channel(false);
    let state_shutdown = receiver.clone();
    let updater = tokio::spawn(ferrix_api::self_update::run_worker(
        state.clone(),
        receiver.clone(),
    ));
    let terminals = tokio::spawn(ferrix_api::deployments::run_terminal_worker(
        state.clone(),
        receiver.clone(),
    ));
    let quotas = tokio::spawn(ferrix_api::deployments::run_quota_worker(
        state.clone(),
        receiver.clone(),
    ));
    let notifications = tokio::spawn(ferrix_api::notifications::run_worker(
        state.clone(),
        receiver.clone(),
    ));
    let housekeeping = tokio::spawn(ferrix_api::housekeeping::run_worker(
        pool.clone(),
        receiver.clone(),
    ));
    let worker = tokio::spawn(ferrix_api::deployments::run_worker(state.clone(), receiver));
    let public = if let Ok(bind) = std::env::var("FERRIX_WEBSITE_BIND") {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let public = ferrix_api::gateway::public_router(state.clone());
        let mut shutdown = state_shutdown;
        Some(tokio::spawn(async move {
            axum::serve(listener, public)
                .with_graceful_shutdown(async move {
                    let _ = shutdown.wait_for(|stop| *stop).await;
                })
                .await
        }))
    } else {
        None
    };
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown().await;
        let _ = worker_shutdown.send(true);
    })
    .await?;
    if let Some(public) = public {
        public.await??;
    }
    worker.await?;
    housekeeping.await?;
    notifications.await?;
    quotas.await?;
    terminals.await?;
    updater.await?;
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

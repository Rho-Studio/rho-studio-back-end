mod config;
mod db;
mod error;
mod routes;
mod state;
mod storage;

use crate::config::AppConfig;
use crate::state::AppState;
use crate::storage::{ObjectStorage, S3Storage};
use axum::Router;
use axum::routing::get;
use redis::aio::ConnectionManager;
use std::sync::Arc;
use std::time::Duration;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Tracing — target uses the CRATE name `rho_studio_back_end`
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,rho_studio_back_end=debug"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().compact())
        .init();

    // Config
    let config = Arc::new(AppConfig::from_env()?);
    tracing::info!(env = %config.app.env, "starting rho-studio-back-end");

    // Database
    let pool = db::init_pool(&config.database.url, config.database.max_connections).await?;
    tracing::info!("connected to postgres");

    db::run_migrations(&pool).await?;
    tracing::info!("migrations applied");

    // Redis
    let redis_client = redis::Client::open(config.redis.url.clone())?;
    let redis = ConnectionManager::new(redis_client).await?;
    tracing::info!("connected to redis");

    // Storage
    let storage: Arc<dyn ObjectStorage> = Arc::new(S3Storage::new(
        &config.storage.endpoint,
        &config.storage.access_key_id,
        &config.storage.secret_access_key,
        &config.storage.bucket,
        &config.storage.region,
        config.storage.gif_cdn_url.clone(),
    )?);
    tracing::info!(
        backend = %config.storage.backend,
        bucket  = %config.storage.bucket,
        "storage initialized"
    );

    // State
    let state = AppState::new(pool, redis, storage, config.clone());

    // Middleware
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let trace = TraceLayer::new_for_http()
        .make_span_with(tower_http::trace::DefaultMakeSpan::new().include_headers(false));

    // Router
    let app = Router::new()
        .route("/health", get(routes::health::health))
        .route("/health/ready", get(routes::health::health_ready))
        .route("/version", get(routes::health::version))
        .route("/api/v1/exercises", get(routes::exercises::get_exercises))
        .route(
            "/api/v1/routines/demo",
            get(routes::routines::get_demo_routine),
        )
        .route("/api/v1/routines/:id", get(routes::routines::get_routine))
        .layer(trace)
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(20 * 1024 * 1024))
        .with_state(state);

    // Serve
    let addr = format!("{}:{}", config.app.host, config.app.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(address = %addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, exiting gracefully");
    tokio::time::sleep(Duration::from_millis(500)).await;
}

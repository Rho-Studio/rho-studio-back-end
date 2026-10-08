/**
 * ============================================================================
 * ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
 * ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
 * ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
 * ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
 * https://rho.studio/
 * ============================================================================
 * File:        main.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-08
 * ============================================================================
 * Description:
 *      Binary entry point and application composition root. Everything the
 *      process needs to serve HTTP is assembled here: tracing, config,
 *      Postgres pool, Redis connection, object storage, application state,
 *      router, shutdown handler.
 *
 *      Boot sequence:
 *          1. Initialize `tracing_subscriber` from `RUST_LOG` (fallback:
 *             `info,rho_studio_fintech=debug`).
 *          2. Load `AppConfig` from environment. Fail if required vars are
 *             missing or invalid.
 *          3. Connect to Postgres via `db::init_pool`. Fail if the pool
 *             cannot be built.
 *          4. Run migrations via `db::run_migrations`. Fail if any
 *             migration fails — the process never serves traffic against a
 *             partially-migrated schema.
 *          5. Connect to Redis via `ConnectionManager`. Fail if
 *             unreachable.
 *          6. Construct the S3-compatible storage backend. Optionally
 *             create the bucket on boot when `STORAGE_AUTO_CREATE_BUCKET`
 *             is set.
 *          7. Assemble `AppState` and pass it to `build_app`.
 *          8. Bind the TCP listener on `APP_HOST:APP_PORT` and serve.
 *
 *      Fail-fast contract:
 *          Every step above returns `anyhow::Result`. The first failure
 *          aborts the process with a non-zero exit and a diagnostic log.
 *          There is no partial-boot state where the router accepts
 *          requests with a missing dependency. See ARCHITECTURE.md
 *          ("Startup sequence").
 *
 *      `build_app`:
 *          Pure function of `(AppState, &AppConfig) -> Router`. Extracted
 *          from `main` so integration tests can build the exact same
 *          router with substitute dependencies — see `route_tests.rs`,
 *          which passes a lazy PgPool, a mock Redis, and a `TestStorage`
 *          fake. Any change to the router shape must be reflected in both
 *          `main` and the test.
 *
 *          Layer ordering matters in Axum: `.layer(...)` applies only to
 *          routes registered before the call. All routes must be declared
 *          above the layer chain. Adding a route after a `.layer()` call
 *          silently omits that layer for the new route.
 *
 *      Middleware stack (outermost first):
 *          - `TraceLayer`             Structured spans for every request.
 *                                     Headers excluded from the span to
 *                                     avoid logging bearer tokens.
 *          - `CorsLayer`              Explicit origin allowlist from
 *                                     config. Allowed methods: GET, POST,
 *                                     OPTIONS. Allowed headers include
 *                                     Authorization, Content-Type,
 *                                     Idempotency-Key, X-Request-Id,
 *                                     X-App-Version, X-Platform. Exposes
 *                                     Retry-After.
 *          - `CompressionLayer`       gzip for responses above the
 *                                     tower-http threshold.
 *          - `RequestBodyLimitLayer`  20 MB cap on request bodies. Sized
 *                                     for the future document-upload
 *                                     endpoint; auth and consent requests
 *                                     are far smaller.
 *
 *      Graceful shutdown:
 *          `shutdown_signal()` waits for SIGINT (Ctrl-C) or SIGTERM. On
 *          Unix, both are handled; on other platforms only Ctrl-C. Once a
 *          signal arrives, `axum::serve` stops accepting connections,
 *          drains in-flight requests, then sleeps 500 ms before returning
 *          to allow the runtime to flush. The `Duration::from_millis(500)`
 *          is a small buffer, not a request timeout.
 *
 *      Bind address:
 *          `APP_HOST:APP_PORT` is passed to `tokio::net::TcpListener::bind`
 *          directly. The default `0.0.0.0:8080` exposes the API on every
 *          interface; in production the process is expected to run behind
 *          a load balancer and the bind is often narrowed to a private
 *          interface by the deployment.
 *
 *      ConnectInfo:
 *          `into_make_service_with_connect_info::<SocketAddr>()` is
 *          required for handlers that use `ConnectInfo<SocketAddr>` — the
 *          auth rate limiter resolves the client IP this way. Removing it
 *          would cause those handlers to fail at runtime with a
 *          missing-extension error.
 *
 *      Design notes:
 *          - `build_app` takes `&AppConfig` in addition to `AppState`
 *            even though `AppState` already carries an `Arc<AppConfig>`.
 *            Historical: `build_app` predates the addition of `config` to
 *            `AppState`. Consolidating to `build_app(state)` is a clean
 *            refactor once all callers are migrated.
 *          - `config.clone()` in the `AppState::new` call is cheap —
 *            `Arc<AppConfig>` refcount bump, not a struct copy.
 *          - The tracing filter defaults to `rho_studio_fintech` as the
 *            module target. Note that crate name dashes become
 *            underscores in Rust module paths — the target is
 *            `rho_studio_fintech`.
 *          - Storage initialization is unconditional except for bucket
 *            creation. There is no feature flag that lets the process boot
 *            without storage; a future configuration might allow that for
 *            auth-only deployments.
 *
 *      Known gaps (tracked in DataModel.md and DEVPLAN.md):
 *          - `build_app` takes both `state` and `config`. Consolidate to
 *            `build_app(state)` once all callers stop needing the raw
 *            config. Low priority.
 *          - No `/metrics` endpoint. Prometheus-style metrics are planned
 *            but not wired.
 *          - No `/version` Git SHA population. The handler exists but the
 *            SHA is optional in the response.
 *          - Swagger / OpenAPI UI is a dependency in `Cargo.toml` but is
 *            not registered. When it is, it must be gated on
 *            `APP_ENV != "production"`.
 *          - The 20 MB `RequestBodyLimitLayer` is global. Per-route limits
 *            would be more appropriate: auth endpoints should cap at
 *            ~100 KB, upload endpoints at 20 MB.
 *          - No request ID middleware. `x-request-id` is accepted in CORS
 *            but not generated or propagated to the trace span.
 *
 *      Not a library. Everything here is private to the binary. Integration
 *      tests interact through `build_app` and the public surface of the
 *      crate's modules.
 * ============================================================================
 */
mod config;
mod db;
mod domain;
mod error;
#[cfg(test)]
mod route_tests;
mod routes;
mod state;
mod storage;

use crate::config::AppConfig;
use crate::state::AppState;
use crate::storage::{ObjectStorage, S3Storage};
use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::{get, post};
use redis::aio::ConnectionManager;
use std::sync::Arc;
use std::time::Duration;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,rho_studio_fintech=debug"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().compact())
        .init();

    // Config
    let config = Arc::new(AppConfig::from_env()?);
    tracing::info!(env = %config.app.env, "starting rho-studio-fintech");

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
    let storage_backend = S3Storage::new(
        &config.storage.endpoint,
        &config.storage.access_key_id,
        &config.storage.secret_access_key,
        &config.storage.bucket,
        &config.storage.region,
    )?;
    if config.storage.auto_create_bucket {
        storage_backend.ensure_bucket().await?;
        tracing::info!(bucket = %config.storage.bucket, "storage bucket initialized");
    }
    let storage: Arc<dyn ObjectStorage> = Arc::new(storage_backend);
    tracing::info!(
        backend = %config.storage.backend,
        bucket  = %config.storage.bucket,
        "storage initialized"
    );

    // State
    let state = AppState::new(pool, redis, storage, config.clone());

    let app = build_app(state, &config)?;

    // Serve
    let addr = format!("{}:{}", config.app.host, config.app.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(address = %addr, "listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

fn build_app(state: AppState, config: &AppConfig) -> anyhow::Result<Router> {
    // Middleware
    let allowed_origins = config
        .cors_allowed_origins
        .iter()
        .map(|origin| origin.parse::<HeaderValue>())
        .collect::<Result<Vec<_>, _>>()?;
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("idempotency-key"),
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-app-version"),
            HeaderName::from_static("x-platform"),
        ])
        .expose_headers([header::RETRY_AFTER]);

    let trace = TraceLayer::new_for_http()
        .make_span_with(tower_http::trace::DefaultMakeSpan::new().include_headers(false));

    Ok(Router::new()
        .route("/health", get(routes::health::health))
        .route("/health/ready", get(routes::health::health_ready))
        .route("/version", get(routes::health::version))
        .route("/api/v1/auth/signup", post(routes::auth::signup))
        .route("/api/v1/auth/login", post(routes::auth::login))
        .route("/api/v1/auth/refresh", post(routes::auth::refresh))
        .route("/api/v1/auth/logout", post(routes::auth::logout))
        .route("/api/v1/auth/logout-all", post(routes::auth::logout_all))
        .route("/api/v1/me", get(routes::auth::me))
        .route("/api/v1/consents/notices",get(routes::consents::get_published_notices))
        .route("/api/v1/me/consents",get(routes::consents::get_my_consents))
        .route("/api/v1/consents", post(routes::consents::record_consent))
        .layer(trace)
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(20 * 1024 * 1024))
        .with_state(state))
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

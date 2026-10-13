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
 * File:        route_tests.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-13
 * ============================================================================
 * Description:
 *      Router-level integration test harness. Boots the full `Router` via
 *      `build_app`, swaps every external dependency for a fast in-process
 *      substitute, and asserts HTTP status codes and response bodies for
 *      auth and consent route boundaries without requiring a database, a
 *      Redis server, or a Docker daemon.
 *
 *      Two test tiers live in this crate:
 *
 *        Tier 1 — this file. Fast, no infrastructure. Boots the router with
 *          a lazy `PgPool` that never connects, an in-process mock Redis,
 *          and a no-op object storage. Runs in milliseconds. Covers
 *          validation, routing, extractor behavior, and auth gating.
 *
 *        Tier 2 — `route_tests/database.rs`. Slow, container-backed.
 *          Spins up a real Postgres via `testcontainers`, applies every
 *          migration, and exercises the full signup + login + refresh +
 *          consent flow. Requires Docker. Run with `--test-threads=1`.
 *
 *      Run this tier:
 *          cargo test route_tests::
 *
 *      Run both tiers:
 *          cargo test -- --test-threads=1
 *
 *      Dependency substitutes:
 *          - PgPool       lazy pool pointed at 127.0.0.1:1. Never opens a
 *                         connection. Any query returns `PoolTimedOut`
 *                         after the acquire timeout. This is deliberate:
 *                         the boundary tests must fail before touching the
 *                         DB, not after.
 *          - Redis        in-process RESP server bound to an ephemeral port.
 *                         Responds to PING, EVAL, and EVALSHA with the
 *                         minimum viable responses for the app's rate
 *                         limiter. Every other command returns `+OK`.
 *          - ObjectStorage `TestStorage`. No-op put/delete, empty get,
 *                         deterministic presigned URL prefix
 *                         (`http://storage.test/{key}`).
 *
 *      Design notes:
 *          - TestApp owns the Redis task handle and aborts it in Drop. This
 *            keeps each test hermetic: the mock server stops the moment the
 *            test function returns and cannot leak into the next test.
 *          - `include_connect_info` on the request helpers inserts a
 *            synthetic `ConnectInfo<SocketAddr>` extension. The real server
 *            gets it from the TCP listener via
 *            `into_make_service_with_connect_info`; `tower::ServiceExt::oneshot`
 *            bypasses the listener, so the extension must be supplied
 *            manually. Endpoints that extract it — `signup`, `login`,
 *            `refresh` — need it set to reach the handler body.
 *          - `test_app_with_pool` is `pub(crate)` because `database.rs`, as
 *            a child module of this file, calls it to build a router bound
 *            to a real container-backed pool.
 *          - The mock Redis parses RESP arrays of bulk strings and produces
 *            replies for the three commands the app actually sends. Adding
 *            a new Redis command to the rate limiter requires extending the
 *            `match` in `serve_mock_redis_connection`.
 *          - Every test creates a fresh `TestApp`, so tests are independent
 *            and safe to run in parallel (no shared mutable state).
 *
 *      Tests covered:
 *          - /health, /health/ready, /version
 *          - /api/v1/auth/login         validation rejection
 *          - /api/v1/auth/refresh       malformed JSON
 *          - /api/v1/auth/logout        malformed body
 *          - /api/v1/auth/logout-all    unauthenticated -> 401
 *          - /api/v1/me                 unauthenticated -> 401
 *          - /api/v1/consents/notices   unauthenticated -> 401
 *          - /api/v1/me/consents        unauthenticated -> 401
 *          - /api/v1/consents           unauthenticated -> 401
 *
 *      Known gaps (tracked in DataModel.md and DevPlan.md):
 *          - No test yet exercises a successful signup or login at this tier
 *            (they require a real DB and live in `database.rs`).
 *          - No test covers CORS preflight behavior.
 *          - No test covers request body size limits.
 *          - No test covers `Content-Type` enforcement.
 *          - No test covers 5xx from a real downstream failure — the lazy
 *            pool always fails at the same point.
 *
 *      Not a serialized type. Not a public API surface. Compiled only under
 *      `#[cfg(test)]`.
 * ============================================================================
 */
use crate::build_app;
use crate::config::{
    AppConfig, AppSection, AuthRateLimitSection, DatabaseSection, JwtSection, RedisSection,
    StorageSection,
};
use crate::state::AppState;
use crate::storage::{ObjectStorage, StorageError};
use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode};
use redis::aio::ConnectionManager;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tower::ServiceExt;

#[cfg(test)]
#[path = "route_tests/database.rs"]
mod database;

struct TestStorage;

#[async_trait]
impl ObjectStorage for TestStorage {
    async fn put_object(
        &self,
        _key: &str,
        _bytes: Vec<u8>,
        _content_type: &str,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn get_object(&self, _key: &str) -> Result<Vec<u8>, StorageError> {
        Ok(Vec::new())
    }

    async fn delete_object(&self, _key: &str) -> Result<(), StorageError> {
        Ok(())
    }

    async fn presigned_get_url(
        &self,
        key: &str,
        _expires_secs: u32,
    ) -> Result<String, StorageError> {
        Ok(format!("http://storage.test/{key}"))
    }

    async fn presigned_put_url(
        &self,
        key: &str,
        _expires_secs: u32,
    ) -> Result<String, StorageError> {
        Ok(format!("http://storage.test/{key}"))
    }

    async fn health_check(&self) -> Result<(), StorageError> {
        Ok(())
    }
}

struct TestApp {
    router: Router,
    redis_task: JoinHandle<()>,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        self.redis_task.abort();
    }
}

async fn test_app() -> TestApp {
    let database_url = "postgres://test:test@127.0.0.1:1/rho_test".to_string();
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy(&database_url)
        .expect("create lazy PostgreSQL pool");

    test_app_with_pool(pool, database_url).await
}

async fn test_app_with_pool(pool: PgPool, database_url: String) -> TestApp {
    let redis_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock Redis listener");
    let redis_address = redis_listener
        .local_addr()
        .expect("read mock Redis address");
    let redis_task = tokio::spawn(serve_mock_redis(redis_listener));
    let redis_client =
        redis::Client::open(format!("redis://{redis_address}/")).expect("create Redis client");
    let redis = ConnectionManager::new(redis_client)
        .await
        .expect("connect to mock Redis");

    let config = Arc::new(AppConfig {
        app: AppSection {
            env: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 0,
        },
        cors_allowed_origins: vec!["http://localhost:3000".to_string()],
        auth_rate_limits: AuthRateLimitSection {
            window_seconds: 900,
            login_attempts: 10,
            refresh_attempts: 30,
            trusted_proxy_cidrs: Vec::new(),
        },
        database: DatabaseSection {
            url: database_url,
            max_connections: 1,
        },
        redis: RedisSection {
            url: format!("redis://{redis_address}/"),
        },
        jwt: JwtSection {
            secret: "test-secret-that-is-long-enough-for-jwt".to_string(),
            access_expiry_seconds: 300,
            refresh_expiry_seconds: 3600,
        },
        storage: StorageSection {
            backend: "test".to_string(),
            auto_create_bucket: false,
            endpoint: "http://storage.test".to_string(),
            access_key_id: "test".to_string(),
            secret_access_key: "test".to_string(),
            bucket: "test".to_string(),
            region: "test".to_string(),
        },
    });
    let storage: Arc<dyn ObjectStorage> = Arc::new(TestStorage);
    let state = AppState::new(pool, redis, storage, config.clone());
    let router = build_app(state, &config).expect("build API router");

    TestApp { router, redis_task }
}

async fn serve_mock_redis(listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        if serve_mock_redis_connection(stream).await.is_err() {
            return;
        }
    }
}

async fn serve_mock_redis_connection(stream: TcpStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    while let Some(command) = read_redis_command(&mut reader).await? {
        let response: &[u8] = match command.first().map(String::as_str) {
            Some("PING") => b"+PONG\r\n",
            Some("EVAL" | "EVALSHA") => b"*2\r\n:1\r\n:60\r\n",
            _ => b"+OK\r\n",
        };
        reader.get_mut().write_all(response).await?;
    }
    Ok(())
}

async fn read_redis_command(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Vec<String>>> {
    let mut line = Vec::new();
    if reader.read_until(b'\n', &mut line).await? == 0 {
        return Ok(None);
    }
    let argument_count = parse_resp_length(&line, b'*')?;
    let mut command = Vec::with_capacity(argument_count);

    for _ in 0..argument_count {
        line.clear();
        reader.read_until(b'\n', &mut line).await?;
        let argument_length = parse_resp_length(&line, b'$')?;
        let mut argument = vec![0; argument_length + 2];
        reader.read_exact(&mut argument).await?;
        command.push(String::from_utf8_lossy(&argument[..argument_length]).into_owned());
    }

    Ok(Some(command))
}

fn parse_resp_length(line: &[u8], prefix: u8) -> io::Result<usize> {
    let length = line
        .strip_prefix(&[prefix])
        .and_then(|length| std::str::from_utf8(length).ok())
        .and_then(|length| length.trim().parse::<usize>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid RESP length"))?;
    Ok(length)
}

async fn request(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&str>,
    include_connect_info: bool,
) -> Response<Body> {
    request_with_headers(router, method, uri, body, include_connect_info, &[]).await
}

async fn request_with_headers(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&str>,
    include_connect_info: bool,
    headers: &[(&str, &str)],
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder
        .body(Body::from(body.unwrap_or_default().to_string()))
        .expect("build API request");
    if include_connect_info {
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:12345"
                .parse::<SocketAddr>()
                .expect("valid peer address"),
        ));
    }
    router
        .clone()
        .oneshot(request)
        .await
        .expect("execute API request")
}

async fn assert_status(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&str>,
    include_connect_info: bool,
    expected: StatusCode,
) {
    let response = request(router, method, uri, body, include_connect_info).await;
    assert_eq!(response.status(), expected, "{uri}");
}

#[tokio::test]
async fn get_health_returns_liveness() {
    let app = test_app().await;
    let response = request(&app.router, Method::GET, "/health", None, false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("parse health body");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["service"], "rho-studio-fintech");
}

#[tokio::test]
async fn get_health_ready_returns_dependency_checks() {
    let app = test_app().await;
    let response = request(&app.router, Method::GET, "/health/ready", None, false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read readiness body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("parse readiness body");
    assert!(value["checks"]["database"].is_boolean());
    assert!(value["checks"]["redis"].is_boolean());
    assert!(value["checks"]["storage"].is_boolean());
}

#[tokio::test]
async fn get_version_returns_package_identity() {
    let app = test_app().await;
    let response = request(&app.router, Method::GET, "/version", None, false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read version body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("parse version body");
    assert_eq!(value["name"], env!("CARGO_PKG_NAME"));
}

#[tokio::test]
async fn post_auth_login_rejects_invalid_credentials_shape() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::POST,
        "/api/v1/auth/login",
        Some(r#"{"email":"","password":"x"}"#),
        true,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

#[tokio::test]
async fn post_auth_refresh_rejects_malformed_json() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::POST,
        "/api/v1/auth/refresh",
        Some("not-json"),
        true,
        StatusCode::BAD_REQUEST,
    )
    .await;
}

#[tokio::test]
async fn post_auth_logout_rejects_malformed_json() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::POST,
        "/api/v1/auth/logout",
        Some("{}"),
        false,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

#[tokio::test]
async fn post_auth_logout_all_requires_authentication() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::POST,
        "/api/v1/auth/logout-all",
        None,
        false,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn get_me_requires_authentication() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::GET,
        "/api/v1/me",
        None,
        false,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn get_consent_notices_requires_authentication() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::GET,
        "/api/v1/consents/notices?locale=es-MX",
        None,
        false,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn get_my_consents_requires_authentication() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::GET,
        "/api/v1/me/consents",
        None,
        false,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn post_consent_requires_authentication() {
    let app = test_app().await;
    assert_status(
        &app.router,
        Method::POST,
        "/api/v1/consents",
        Some(r#"{"purpose_code":"privacy","locale":"es-MX","version":"1","action":"granted"}"#),
        false,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

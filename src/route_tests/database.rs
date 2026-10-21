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
 * File:        route_tests/database.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-21
 * ============================================================================
 * Description:
 *      Container-backed integration tests for the authentication and
 *      consent surfaces. Every test in this file boots a real PostgreSQL
 *      instance via `testcontainers`, applies the full migration set, and
 *      drives the production router through the full request/response
 *      cycle. This is the tier that catches schema drift, constraint
 *      violations, transaction isolation issues, and any bug that a mock
 *      cannot reproduce.
 *
 *      Run this tier:
 *          cargo test route_tests::database:: -- --test-threads=1 --nocapture
 *
 *      Docker must be running. The container is created, used, and
 *      destroyed per test; the developer's dev database and dev Redis are
 *      never touched.
 *
 *      Test environment:
 *          - Postgres      postgres:16-alpine, ephemeral per test, on an
 *                          OS-assigned host port. `TestEnv` owns the
 *                          container handle and drops it when the test
 *                          function returns.
 *          - Redis         the in-process mock defined in the parent
 *                          module (`route_tests.rs`). Same response shape
 *                          the rate limiter expects.
 *          - ObjectStorage `TestStorage` from the parent module. No-op.
 *
 *      Test isolation:
 *          - Each test gets its own container and its own database. Tests
 *            cannot see or corrupt each other's data.
 *          - Emails are UUID-suffixed so tests are rerunnable without
 *            manual cleanup.
 *          - The container lives as long as the `TestEnv` binding. The
 *            pattern `let TestEnv { app, pool, _postgres } = start_test_env()`
 *            binds `_postgres` as a real field, so the container stays
 *            alive until the test ends. Using `..` instead would drop the
 *            container immediately and every query would fail with
 *            `UnexpectedEof` or `PoolTimedOut`.
 *
 *      Tests in this file:
 *
 *        postgres_backed_signup_creates_user_and_rejects_duplicate
 *          - Signup returns 200 with a valid access and refresh token
 *            pair.
 *          - Response `user.email` matches the submitted email.
 *          - The row lands in `users` with an Argon2id hash (PHC prefix
 *            `$argon2`), never plaintext.
 *          - Exactly one `refresh_tokens` row was created for the
 *            auto-login session.
 *          - A duplicate signup with the same email returns 422 and does
 *            not create a second `users` row.
 * 
 *          postgres_backed_login_rejects_invalid_password_and_inactive_user
 *          - Setup seeds a verified, active user.
 *          - Attempt login with an incorrect password.
 *          - Returns 401 Unauthorized with the generic public error message
 *            "Authentication failed" (mitigating user enumeration).
 *          - Manually sets `is_active = false` on the user.
 *          - Attempting login with the correct password returns 401
 *            Unauthorized, failing closed at the SQL query level.
 * 
 *          postgres_backed_refresh_rejects_expired_token
 *          - Setup seeds a user and logs in.
 *          - Manually forces the database row's `expires_at` into the past.
 *          - Attempting to refresh returns 401.
 *          - The specific expired token row is marked as revoked.
 *
 *        postgres_backed_refresh_rejects_inactive_user_and_revokes_family
 *          - Setup seeds a user and logs in.
 *          - Manually sets `is_active = false` on the user.
 *          - Attempting to refresh returns 401.
 *          - The entire token family is revoked as a security measure.
 *
 *        postgres_backed_refresh_rejects_unverified_user_and_revokes_family
 *          - Setup seeds a user and logs in.
 *          - Manually sets `email_verified = false` on the user.
 *          - Attempting to refresh returns 401.
 *          - The entire token family is revoked as a security measure.
 *
 *        postgres_backed_refresh_rejects_access_token_confusion
 *          - Setup seeds a user and logs in.
 *          - Attempts to use an Access Token in the `refresh_token` field.
 *          - Returns 401 at the JWT decoding step (fails before touching
 *            the database).
 *          - Verifies that no tokens were revoked.
 *
 *        postgres_backed_auth_refresh_logout_profile_and_consent_flow
 *          - Login with a case-insensitive email
 *            (`ROUTE-TEST@example.test` matches the seeded
 *            `route-test@example.test`).
 *          - `/me` returns the authenticated user.
 *          - Consent notice list returns the seeded published notice.
 *          - Initial consent state is empty.
 *          - Grant / grant / withdraw / withdraw are idempotent: the
 *            event count is 1, 1, 2, 2 respectively.
 *          - The current consent state reflects the last action
 *            (`withdrawn`) and the notice version.
 *          - Refresh rotates the refresh token: a new token is issued and
 *            the previous row is revoked.
 *          - Replaying the revoked refresh token returns 401 and revokes
 *            the entire token family: no active tokens remain in the
 *            family.
 *          - The rotated access token is now dead — `/me` returns 401 —
 *            because the family-wide revocation check in
 *            `authenticated_user_id` catches it.
 *          - A fresh login creates a new family; logout revokes that
 *            family; the access token from that session is rejected on
 *            `/me` afterward.
 *
 *      Design notes:
 *          - `start_test_env` is the single entry point for a container-
 *            backed test. It never leaks the container handle: the
 *            struct binding pattern at each call site is what keeps it
 *            alive.
 *          - `connect_with_retry` masks the race between the container
 *            reporting "ready" and the wire protocol actually accepting
 *            connections. 30 attempts at 1 s intervals with a 30 s
 *            acquire timeout is a generous margin. If the pattern needs
 *            to be tightened, increase the retry budget, not the
 *            acquire timeout alone.
 *          - `POSTGRES_PORT` is a plain `u16` upgraded to a
 *            `ContainerPort` via `.tcp()` at the call site. The
 *            `IntoContainerPort` trait must be in scope for that to
 *            compile — it is imported at the top of the file.
 *          - The migration runner is invoked as `db::run_migrations(&pool)`
 *            so the exact same migration sequence the production binary
 *            uses runs against every fresh container.
 *          - Emails are UUID-suffixed. Parallel test execution is safe
 *            even though the current recommendation is
 *            `--test-threads=1` for resource reasons.
 *
 *      Testing gaps (tracked in DataModel.md and DevPlan.md):
 *          - No test covers the `kid`-based JWT signing key rotation
 *            (Feature F2, later implementation).
 *          - No test covers email verification or password reset
 *            (Feature F2, later implementation).
 *          - No test covers session listing or per-session revocation
 *            (Feature F2, later implementation).
 *          - No test covers the customer ↔ user linkage step after KYC
 *            (Feature F3).
 *          - No test covers the ledger or any money-movement path
 *            (Features F7, F8).
 *          - No test covers RLS enforcement: no handler sets
 *            `app.current_customer_id` today, so the policies in
 *            migration 9 are dormant.
 *
 *      Not a serialized type. Not a public API surface. Compiled only under
 *      `#[cfg(test)]` as a child module of `route_tests.rs`.
 * ============================================================================
 */
use super::{TestApp, request_with_headers, test_app_with_pool};
use crate::db;
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use axum::{body::to_bytes, http::StatusCode};
use rand::rngs::OsRng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};
use uuid::Uuid;

const POSTGRES_PORT: u16 = 5432;

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("response is valid JSON")
}

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

// ---------------------------------------------------------------------------
// Container bootstrap
// ---------------------------------------------------------------------------

/// A running test environment: isolated Postgres + the real app router.
/// Dropping this stops and removes the container.
struct TestEnv {
    app: TestApp,
    pool: PgPool,
    _postgres: ContainerAsync<GenericImage>,
}

async fn start_test_env() -> TestEnv {
    // 0.15+ API: no Cli, no docker.run(); use AsyncRunner + ImageExt.
    let postgres = GenericImage::new("postgres", "16-alpine")
        .with_exposed_port(POSTGRES_PORT.tcp()) // u16 -> ContainerPort
        .with_wait_for(WaitFor::message_on_stdout(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_USER", "rho_test")
        .with_env_var("POSTGRES_PASSWORD", "rho_test")
        .with_env_var("POSTGRES_DB", "rho_test")
        .start()
        .await
        .expect("start postgres container");

    let host_port = postgres
        .get_host_port_ipv4(POSTGRES_PORT)
        .await
        .expect("resolve mapped container port");

    let database_url =
        format!("postgres://rho_test:rho_test@127.0.0.1:{host_port}/rho_test?sslmode=disable");

    let pool = connect_with_retry(&database_url).await;
    db::run_migrations(&pool)
        .await
        .expect("apply all migrations to isolated test database");

    let app = test_app_with_pool(pool.clone(), database_url).await;

    TestEnv {
        app,
        pool,
        _postgres: postgres,
    }
}

async fn connect_with_retry(database_url: &str) -> PgPool {
    let mut last_error = None;
    for _ in 0..30 {
        match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect(database_url)
            .await
        {
            Ok(pool) => return pool,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    panic!("connect to isolated PostgreSQL test container: {last_error:?}");
}

// ---------------------------------------------------------------------------
// Signup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn postgres_backed_signup_creates_user_and_rejects_duplicate() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;

    let email = format!("signup-{}@example.test", Uuid::new_v4());
    let password = "correct-horse-battery-staple";

    let body = json!({
        "email": email,
        "password": password,
        "display_name": "Signup Tester"
    })
    .to_string();

    let response = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/signup",
        Some(&body),
        true, // signup extracts ConnectInfo<SocketAddr>
        &[],
    )
    .await;
    //assert_eq!(response.status(), StatusCode::OK, "signup should succeed");
    let status = response.status();
    if status != StatusCode::OK {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        panic!(
            "signup failed: status={} body={}",
            status,
            String::from_utf8_lossy(&bytes)
        );
    }

    let payload = json_body(response).await;
    assert!(payload["access_token"].as_str().is_some_and(|t| !t.is_empty()));
    assert!(payload["refresh_token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(payload["token_type"], "Bearer");
    assert_eq!(payload["user"]["email"], email);
    assert_eq!(payload["user"]["display_name"], "Signup Tester");

    // Row landed in the DB; password is Argon2-hashed and never plaintext.
    let (user_id, stored_hash, verified, active): (Uuid, String, bool, bool) = sqlx::query_as(
        "SELECT id, password_hash, email_verified, is_active FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("signup created user row");

    assert!(stored_hash.starts_with("$argon2"), "got: {stored_hash}");
    assert_ne!(stored_hash, password);
    assert!(verified && active);

    // Exactly one refresh token persisted for the auto-login session.
    let refresh_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM refresh_tokens WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(refresh_count, 1);

    // Duplicate signup must be rejected and must not create a second row.
    let dup = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/signup",
        Some(&json!({ "email": email, "password": password }).to_string()),
        true,
        &[],
    )
    .await;
    assert_eq!(dup.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "duplicate signup must not create a second user");
}

/*---------------------------------------------------------------------------
*   Login checks
* ---------------------------------------------------------------------------*/

#[tokio::test]
async fn postgres_backed_login_rejects_invalid_password_and_inactive_user() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;

    // 1. Seed a verified, active user
    let user_id = Uuid::new_v4();
    let email = format!("login-test-{}@example.test", user_id);
    let password = "correct-horse-battery-staple";
    
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash password")
        .to_string();
        
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, email_verified, is_active)
         VALUES ($1, $2, $3, true, true)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&password_hash)
    .execute(&pool)
    .await
    .unwrap();

    // 2. Attempt login with WRONG password
    let bad_login = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/login",
        Some(&json!({
            "email": email,
            "password": "wrong-password"
        }).to_string()),
        true,
        &[],
    ).await;
    
    // Assert 401 Unauthorized
    assert_eq!(bad_login.status(), StatusCode::UNAUTHORIZED, "Wrong password must be rejected");
    let body = json_body(bad_login).await;
    assert_eq!(body["error"]["message"], "Authentication failed");

    // 3. Deactivate the user in the DB
    sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();

    // 4. Attempt login with the CORRECT password but inactive account
    let inactive_login = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/login",
        Some(&json!({
            "email": email,
            "password": password
        }).to_string()),
        true,
        &[],
    ).await;

    // Assert 401 Unauthorized (fails closed at the SQL level)
    assert_eq!(inactive_login.status(), StatusCode::UNAUTHORIZED, "Inactive user must be rejected");
}

/*---------------------------------------------------------------------------
*   Refresh Edge Cases
* ---------------------------------------------------------------------------*/

/// Helper to reduce duplication in the refresh edge-case tests.
/// Seeds a verified user, logs them in, and returns the tokens.
async fn seed_and_login(app: &TestApp, pool: &PgPool) -> (Uuid, String, String) {
    let user_id = Uuid::new_v4();
    let email = format!("refresh-test-{}@example.test", user_id);
    let password = "integration-test-password";
    
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash test password")
        .to_string();

    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash, email_verified, is_active)
         VALUES ($1, $2, $3, $4, true, true)",
    )
    .bind(user_id)
    .bind(&email)
    .bind("Refresh Test User")
    .bind(password_hash)
    .execute(pool)
    .await
    .expect("seed verified test user");

    let login_response = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/login",
        Some(&json!({
            "email": email,
            "password": password
        }).to_string()),
        true,
        &[],
    )
    .await;
    assert_eq!(login_response.status(), StatusCode::OK);

    let login = json_body(login_response).await;
    let access_token = login["access_token"].as_str().unwrap().to_string();
    let refresh_token = login["refresh_token"].as_str().unwrap().to_string();

    (user_id, access_token, refresh_token)
}

#[tokio::test]
async fn postgres_backed_refresh_rejects_expired_token() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;
    
    // 1. Setup: Seed a user, login, and get the refresh token
    let (_user_id, _access_token, refresh_token) = seed_and_login(&app, &pool).await;
    let token_hash = hash_token(&refresh_token);

    // 2. Manipulate DB: Force the token to be expired
    // We must push created_at into the past as well to satisfy:
    // CHECK (expires_at > created_at)
    sqlx::query(
        "UPDATE refresh_tokens 
         SET created_at = now() - INTERVAL '2 hours', 
             expires_at = now() - INTERVAL '1 hour' 
         WHERE token_hash = $1"
    )
    .bind(&token_hash)
    .execute(&pool)
    .await
    .expect("fail to expire token");

    // 3. Attempt refresh
    let body = json!({ "refresh_token": refresh_token }).to_string();
    let response = request_with_headers(
        &app.router, axum::http::Method::POST, "/api/v1/auth/refresh", Some(&body), true, &[]
    ).await;

    // 4. Assert 401
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 5. Assert the specific row is now revoked
    let is_revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM refresh_tokens WHERE token_hash = $1"
    )
    .bind(&token_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(is_revoked, "Expired token should be marked as revoked");
}

#[tokio::test]
async fn postgres_backed_refresh_rejects_inactive_user_and_revokes_family() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;
    
    let (user_id, _access_token, refresh_token) = seed_and_login(&app, &pool).await;
    let token_hash = hash_token(&refresh_token);

    let family_id: Uuid = sqlx::query_scalar(
        "SELECT token_family_id FROM refresh_tokens WHERE token_hash = $1"
    )
    .bind(&token_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 1. Manipulate DB: Deactivate the user
    sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("fail to deactivate user");

    // 2. Attempt refresh
    let body = json!({ "refresh_token": refresh_token }).to_string();
    let response = request_with_headers(
        &app.router, axum::http::Method::POST, "/api/v1/auth/refresh", Some(&body), true, &[]
    ).await;

    // 3. Assert 401
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 4. Assert the *whole family* is revoked
    let active_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM refresh_tokens WHERE token_family_id = $1 AND revoked_at IS NULL"
    )
    .bind(family_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    
    assert_eq!(active_tokens, 0, "Inactive user should trigger family-wide revocation");
}

#[tokio::test]
async fn postgres_backed_refresh_rejects_unverified_user_and_revokes_family() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;
    
    let (user_id, _access_token, refresh_token) = seed_and_login(&app, &pool).await;
    let token_hash = hash_token(&refresh_token);

    let family_id: Uuid = sqlx::query_scalar(
        "SELECT token_family_id FROM refresh_tokens WHERE token_hash = $1"
    )
    .bind(&token_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 1. Manipulate DB: Unverify the user
    sqlx::query("UPDATE users SET email_verified = false WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("fail to unverify user");

    // 2. Attempt refresh
    let body = json!({ "refresh_token": refresh_token }).to_string();
    let response = request_with_headers(
        &app.router, axum::http::Method::POST, "/api/v1/auth/refresh", Some(&body), true, &[]
    ).await;

    // 3. Assert 401
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 4. Assert the *whole family* is revoked
    let active_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM refresh_tokens WHERE token_family_id = $1 AND revoked_at IS NULL"
    )
    .bind(family_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    
    assert_eq!(active_tokens, 0, "Unverified user should trigger family-wide revocation");
}

#[tokio::test]
async fn postgres_backed_refresh_rejects_access_token_confusion() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;
    
    // 1. Setup: Get BOTH tokens
    let (_user_id, access_token, _refresh_token) = seed_and_login(&app, &pool).await;

    // 2. Attempt refresh using the ACCESS token
    let body = json!({ "refresh_token": access_token }).to_string();
    let response = request_with_headers(
        &app.router, axum::http::Method::POST, "/api/v1/auth/refresh", Some(&body), true, &[]
    ).await;

    // 3. Assert 401
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 4. Assert that no tokens were revoked (it should fail at the JWT decode step, before DB lookup)
    let active_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM refresh_tokens WHERE revoked_at IS NULL"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    
    assert_eq!(active_tokens, 1, "Token type confusion should fail before touching the DB");
}

// ---------------------------------------------------------------------------
// Full auth + consent flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn postgres_backed_auth_refresh_logout_profile_and_consent_flow() {
    let TestEnv { app, pool, _postgres } = start_test_env().await;

    // --- Seed a verified user -------------------------------------------------
    let user_id = Uuid::new_v4();
    let password = "integration-test-password";
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash integration-test password")
        .to_string();
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash, email_verified, is_active)
         VALUES ($1, $2, $3, $4, true, true)",
    )
    .bind(user_id)
    .bind("route-test@example.test")
    .bind("Route Test User")
    .bind(password_hash)
    .execute(&pool)
    .await
    .expect("seed verified test user");

    // --- Seed a published consent notice -------------------------------------
    let notice_id = Uuid::new_v4();
    let notice_content = "Integration-test privacy notice";
    let notice_hash = format!("{:x}", Sha256::digest(notice_content.as_bytes()));
    sqlx::query(
        r#"
        INSERT INTO consent_notices (
            id, purpose_code, locale, version, content, content_sha256,
            status, published_at
        )
        VALUES ($1, 'privacy', 'es-MX', 'test-v1', $2, $3, 'published', now())
        "#,
    )
    .bind(notice_id)
    .bind(notice_content)
    .bind(notice_hash)
    .execute(&pool)
    .await
    .expect("seed published test notice");

    // --- Login (case-insensitive email lookup) -------------------------------
    let login_response = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/login",
        Some(
            &json!({
                "email": "ROUTE-TEST@example.test",
                "password": password
            })
            .to_string(),
        ),
        true,
        &[],
    )
    .await;
    assert_eq!(login_response.status(), StatusCode::OK);
    let login = json_body(login_response).await;
    let access_token = login["access_token"].as_str().expect("access token").to_string();
    let refresh_token = login["refresh_token"]
        .as_str()
        .expect("refresh token")
        .to_string();
    assert_eq!(login["user"]["id"], user_id.to_string());
    assert_eq!(login["user"]["email"], "route-test@example.test");

    let first_token_hash = hash_token(&refresh_token);
    let family_id: Uuid =
        sqlx::query_scalar("SELECT token_family_id FROM refresh_tokens WHERE token_hash = $1")
            .bind(&first_token_hash)
            .fetch_one(&pool)
            .await
            .expect("login persisted hashed refresh token");

    let bearer = format!("Bearer {access_token}");

    // --- /me ----------------------------------------------------------------
    let profile_response = request_with_headers(
        &app.router,
        axum::http::Method::GET,
        "/api/v1/me",
        None,
        false,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(profile_response.status(), StatusCode::OK);
    let profile = json_body(profile_response).await;
    assert_eq!(profile["id"], user_id.to_string());
    assert_eq!(profile["display_name"], "Route Test User");

    // --- Consent notices -----------------------------------------------------
    let notice_response = request_with_headers(
        &app.router,
        axum::http::Method::GET,
        "/api/v1/consents/notices?locale=es-MX",
        None,
        false,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(notice_response.status(), StatusCode::OK);
    let notices = json_body(notice_response).await;
    assert_eq!(notices.as_array().expect("array").len(), 1);
    assert_eq!(notices[0]["id"], notice_id.to_string());

    let initial_consents = request_with_headers(
        &app.router,
        axum::http::Method::GET,
        "/api/v1/me/consents",
        None,
        false,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(initial_consents.status(), StatusCode::OK);
    assert_eq!(json_body(initial_consents).await, json!([]));

    // --- Idempotent consent grant/withdraw -----------------------------------
    for (action, expected_events) in [
        ("granted", 1_i64),
        ("granted", 1),
        ("withdrawn", 2),
        ("withdrawn", 2),
    ] {
        let request_body = json!({
            "purpose_code": "privacy",
            "locale": "es-MX",
            "version": "test-v1",
            "action": action
        })
        .to_string();
        let response = request_with_headers(
            &app.router,
            axum::http::Method::POST,
            "/api/v1/consents",
            Some(&request_body),
            false,
            &[("authorization", &bearer)],
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "action={action}");

        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM customer_consent_events WHERE user_id = $1 AND notice_id = $2",
        )
        .bind(user_id)
        .bind(notice_id)
        .fetch_one(&pool)
        .await
        .expect("count persisted consent events");
        assert_eq!(event_count, expected_events, "action={action}");
    }

    let consent_state = request_with_headers(
        &app.router,
        axum::http::Method::GET,
        "/api/v1/me/consents",
        None,
        false,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(consent_state.status(), StatusCode::OK);
    let consent_state = json_body(consent_state).await;
    assert_eq!(consent_state[0]["action"], "withdrawn");
    assert_eq!(consent_state[0]["version"], "test-v1");

    // --- Refresh rotation ----------------------------------------------------
    let refresh_response = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/refresh",
        Some(&json!({ "refresh_token": refresh_token }).to_string()),
        true,
        &[],
    )
    .await;
    assert_eq!(refresh_response.status(), StatusCode::OK);
    let rotated = json_body(refresh_response).await;
    let rotated_access = rotated["access_token"]
        .as_str()
        .expect("rotated access token")
        .to_string();
    let rotated_refresh = rotated["refresh_token"]
        .as_str()
        .expect("rotated refresh token")
        .to_string();
    assert_ne!(rotated_refresh, refresh_token);

    let old_token_revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM refresh_tokens WHERE token_hash = $1",
    )
    .bind(&first_token_hash)
    .fetch_one(&pool)
    .await
    .expect("find rotated refresh token");
    assert!(old_token_revoked, "rotation revokes the old refresh token");

    // --- Replay detection revokes the family --------------------------------
    let replay_response = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/refresh",
        Some(&json!({ "refresh_token": refresh_token }).to_string()),
        true,
        &[],
    )
    .await;
    assert_eq!(replay_response.status(), StatusCode::UNAUTHORIZED);

    let family_active_tokens: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM refresh_tokens WHERE token_family_id = $1 AND revoked_at IS NULL",
    )
    .bind(family_id)
    .fetch_one(&pool)
    .await
    .expect("count active tokens after replay");
    assert_eq!(family_active_tokens, 0, "replay revokes the token family");

    // The rotated access token is now dead too (family revoked).
    let rotated_bearer = format!("Bearer {rotated_access}");
    let revoked_profile = request_with_headers(
        &app.router,
        axum::http::Method::GET,
        "/api/v1/me",
        None,
        false,
        &[("authorization", &rotated_bearer)],
    )
    .await;
    assert_eq!(revoked_profile.status(), StatusCode::UNAUTHORIZED);

    // --- Fresh login + logout -----------------------------------------------
    let second_login = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/login",
        Some(
            &json!({
                "email": "route-test@example.test",
                "password": password
            })
            .to_string(),
        ),
        true,
        &[],
    )
    .await;
    assert_eq!(second_login.status(), StatusCode::OK);
    let second_session = json_body(second_login).await;
    let second_refresh = second_session["refresh_token"]
        .as_str()
        .expect("second refresh token")
        .to_string();
    let second_access = second_session["access_token"]
        .as_str()
        .expect("second access token")
        .to_string();
    let second_token_hash = hash_token(&second_refresh);
    let second_family: Uuid =
        sqlx::query_scalar("SELECT token_family_id FROM refresh_tokens WHERE token_hash = $1")
            .bind(&second_token_hash)
            .fetch_one(&pool)
            .await
            .expect("second login persisted refresh token");

    let logout_response = request_with_headers(
        &app.router,
        axum::http::Method::POST,
        "/api/v1/auth/logout",
        Some(&json!({ "refresh_token": second_refresh }).to_string()),
        false,
        &[],
    )
    .await;
    assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);

    let second_family_active_tokens: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM refresh_tokens WHERE token_family_id = $1 AND revoked_at IS NULL",
    )
    .bind(second_family)
    .fetch_one(&pool)
    .await
    .expect("count active tokens after logout");
    assert_eq!(second_family_active_tokens, 0, "logout revokes its token family");

    let second_bearer = format!("Bearer {second_access}");
    let logged_out_profile = request_with_headers(
        &app.router,
        axum::http::Method::GET,
        "/api/v1/me",
        None,
        false,
        &[("authorization", &second_bearer)],
    )
    .await;
    assert_eq!(logged_out_profile.status(), StatusCode::UNAUTHORIZED);
}
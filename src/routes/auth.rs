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
 * File:        src/routes/auth.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-12
 * ============================================================================
 * Description:
 *      Authentication and session management handlers. Implements the
 *      identity surface of the API: registration, login, refresh-token
 *      rotation, logout, logout-all, and the current-user profile. This
 *      is the only module whose handlers are fully wired to the router
 *      and covered by container-backed integration tests today.
 *
 *      Endpoints:
 *          POST /api/v1/auth/signup      Register + auto-login
 *          POST /api/v1/auth/login       Email + password -> token pair
 *          POST /api/v1/auth/refresh     Rotate refresh token
 *          POST /api/v1/auth/logout      Revoke caller's token family
 *          POST /api/v1/auth/logout-all  Revoke every session for the user
 *          GET  /api/v1/me               Current user profile (Bearer)
 *
 *      Token model (current implementation):
 *          Two JWTs per session, both HS256, both sharing the same
 *          `session_id` (the `token_family_id` in the DB):
 *
 *              Access   short-lived (`access_expiry_seconds`), stateless,
 *                       sent in `Authorization: Bearer <token>`.
 *              Refresh  long-lived (`refresh_expiry_seconds`), stored
 *                       server-side as SHA-256 hash only.
 *
 *          Claims: sub (user id), session_id, exp, iat, jti, iss, aud,
 *          token_use. Issuer is `rho-studio-fintech`, audience is
 *          `rho-studio-fintech-mobile`. `decode_token` pins the algorithm
 *          and validates issuer, audience, expiry, and `token_use`.
 *
 *      Token model (target, per architecture.md):
 *          Migration to OIDC + OAuth 2.0 Authorization Code Flow with PKCE
 *          for native mobile clients. HS256 is replaced by ES256 or PS256
 *          asymmetric signing with a JWKS endpoint and `kid`-based key
 *          rotation. Sender-constrained tokens via mTLS or DPoP, FAPI
 *          alignment, and mandatory MFA for step-up flows. The current
 *          handler shape is deliberately close to the target so the
 *          migration is a substitution rather than a rewrite: `issue_token`
 *          becomes a call into an authorization-server interface, and
 *          `authenticated_user_id` becomes a resource-server validator
 *          that additionally checks the `cnf` confirmation claim.
 *
 *      Refresh rotation and reuse detection:
 *          Every `/auth/refresh` revokes the presented row and inserts a
 *          new row in the same family. Presenting an already-revoked
 *          token triggers family-wide revocation — every token in the
 *          session dies, forcing re-authentication. This limits the
 *          blast radius of a stolen refresh token to at most one refresh
 *          cycle. The detection logic lives in `refresh`.
 *
 *      Session liveness:
 *          Access tokens are stateless. To make `logout-all` immediately
 *          effective, `authenticated_user_id` performs a per-request
 *          existence check for a live refresh token in the token's
 *          family. If the family is fully revoked, the access token is
 *          rejected even though its signature and expiry are still
 *          valid. This costs one indexed lookup per authenticated
 *          request — the partial index in migration 10 makes it cheap.
 *
 *      Password storage:
 *          Argon2id via the `argon2` crate with default parameters
 *          (RFC 9106: m=19456 KiB, t=2, p=1). Per-user random salt. PHC
 *          string stored in `users.password_hash`. Plaintext never
 *          logged.
 *
 *      Timing-attack mitigation:
 *          `login` verifies the submitted password against a lazy-
 *          initialized dummy hash when the email is not found. Both the
 *          found and not-found code paths take similar time, so an
 *          attacker cannot distinguish "user does not exist" from "wrong
 *          password" by response latency.
 *
 *      Rate limiting:
 *          Redis-backed fixed window with atomic INCR + EXPIRE via Lua.
 *          `signup` is limited per (IP, email); `login` per (IP, email);
 *          `refresh` per (IP, token_hash). Subjects are SHA-256 hashed
 *          before becoming Redis keys, so raw emails and tokens never
 *          land in the cache. On Redis failure the handler returns
 *          `AppError::Unavailable` (503) — fail-closed.
 *
 *      Client IP resolution:
 *          `client_ip` implements the RFC 7239 reverse-proxy pattern.
 *          The TCP peer address is used directly unless it falls inside
 *          a configured `trusted_proxy_cidrs` network. When trusted,
 *          `X-Forwarded-For` is parsed right-to-left and the first
 *          address outside the trusted set is treated as the client.
 *          Left entries in the header are client-supplied and can be
 *          spoofed; right-to-left traversal is what makes this safe.
 *
 *      Design notes:
 *          - `refresh_tokens.id` is generated by `issue_token` (the JWT
 *            `jti`), not by the database. The row PK and the token claim
 *            must match; the "look up by jti" pattern depends on it.
 *          - `token_hash` uses SHA-256, not Argon2. Refresh tokens are
 *            high-entropy random JWTs, not user-chosen passwords — a
 *            fast hash is sufficient and keeps the refresh hot path
 *            cheap.
 *          - `logout` always returns 204 and swallows DB errors. It is
 *            idempotent by design: revoking an already-revoked or
 *            nonexistent token is not an error the client needs to know
 *            about. It also accepts expired refresh tokens (via
 *            `allow_expired = true`) so a stale token can still kill
 *            its family.
 *          - `logout_all` requires a valid access token and revokes
 *            every refresh token for the user. The session-liveness
 *            check makes it effective for outstanding access tokens
 *            immediately.
 *          - Validation errors on signup return a generic message so an
 *            attacker cannot use the response shape to probe field
 *            constraints beyond what the mobile client already knows.
 *          - `user_agent` is truncated to 512 chars before insert to
 *            match the column width; otherwise a long UA string would
 *            fail the INSERT with a Postgres error.
 *          - The `argon2::password_hash::Error` type does not implement
 *            `std::error::Error`, so it cannot be converted directly
 *            with `?` into `AppError::Internal`. The signup handler
 *            wraps it with `anyhow::anyhow!(error.to_string())`.
 *          - The `eprintln!` in the signup INSERT error branch is a
 *            temporary dev diagnostic. Replace with `tracing::error!`
 *            before production; a structured log entry is required for
 *            audit and observability.
 *
 *      Testing:
 *          Unit tests in this file cover the pure helpers:
 *          `token_hash`, `rate_limit_key`, `client_ip`, and
 *          `verify_password`. Integration tests in `route_tests.rs` and
 *          `route_tests/database.rs` exercise the full flow against a
 *          real Postgres: signup, duplicate rejection, login with
 *          mixed-case email, refresh rotation, replay detection,
 *          family-wide revocation, logout, and `/me`.
 *
 *      Known gaps (tracked in DataModel.md and DevPlan.md):
 *          - P0: `email_verified` is set to `true` in `signup` for dev
 *            convenience. Production must flip to `false` and add a
 *            verification flow (HU-002).
 *          - P0: `JWT_SECRET` is a single static secret; no `kid`-based
 *            rotation. Rotating invalidates every outstanding token
 *            (HU-007).
 *          - P0: no password strength policy beyond length >= 8. No
 *            breach-list check, no complexity rule (HU-010).
 *          - P0: no password reset flow (`/auth/forgot-password`,
 *            `/auth/reset-password`, `password_reset_tokens` table).
 *            HU-009.
 *          - P1: no session listing (`GET /me/sessions`) or per-session
 *            revocation (`DELETE /me/sessions/:id`). HU-006 scope.
 *          - P1: no password change endpoint requiring the current
 *            password. HU-009 scope.
 *          - P1: no refresh-token absolute max lifetime; rotation
 *            extends a session indefinitely. Target is a 90-day
 *            absolute cap.
 *          - P1: rate limiting covers only auth endpoints. Transfer
 *            submission, KYC upload, consent recording, and ARCO
 *            request creation have no per-user quota. HU-008 scope.
 *          - P2: no login notification on new device.
 *          - Target: migrate to OIDC + OAuth 2.0 + PKCE with ES256 or
 *            PS256 signing, JWKS, mTLS-bound tokens, and mandatory MFA
 *            for step-up operations. See architecture.md, section
 *            "Authentication and Authorization Architecture."
 *
 *      Not a serialized type. The response types (`TokenResponse`,
 *      `UserResponse`) implement `Serialize`; the internal types
 *      (`UserRecord`, `StoredRefreshToken`, `IssuedToken`, `TokenClaims`)
 *      do not cross the API boundary.
 * ============================================================================
 */
use crate::config::AuthRateLimitSection;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Postgres, Transaction};
use std::{
    net::{IpAddr, SocketAddr},
    sync::OnceLock,
};
use uuid::Uuid;

const TOKEN_ISSUER: &str = "rho-studio-fintech";
const TOKEN_AUDIENCE: &str = "rho-studio-fintech-mobile";
const MAX_EMAIL_LENGTH: usize = 320;
const MAX_PASSWORD_LENGTH: usize = 1024;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TokenUse {
    Access,
    Refresh,
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenClaims {
    sub: String,
    session_id: Uuid,
    exp: usize,
    iat: usize,
    jti: String,
    iss: String,
    aud: String,
    token_use: TokenUse,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    email: String,
    password: String,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    access_token: String,
    refresh_token: String,
    token_type: &'static str,
    expires_in: i64,
    user: UserResponse,
}

#[derive(Debug, FromRow, Serialize)]
pub struct UserResponse {
    id: Uuid,
    email: String,
    display_name: Option<String>,
}

#[derive(Debug, FromRow)]
struct UserRecord {
    id: Uuid,
    email: String,
    display_name: Option<String>,
    password_hash: String,
}

#[derive(Debug, FromRow)]
struct StoredRefreshToken {
    id: Uuid,
    user_id: Uuid,
    token_family_id: Uuid,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}


struct IssuedToken {
    value: String,
    id: Uuid,
    expires_at: DateTime<Utc>,
}

fn signing_key(state: &AppState) -> EncodingKey {
    EncodingKey::from_secret(state.config.jwt.secret.as_bytes())
}

fn decoding_key(state: &AppState) -> DecodingKey {
    DecodingKey::from_secret(state.config.jwt.secret.as_bytes())
}

fn issue_token(
    state: &AppState,
    user_id: Uuid,
    session_id: Uuid,
    token_use: TokenUse,
    lifetime_seconds: i64,
) -> AppResult<IssuedToken> {
    let now = Utc::now();
    let expires_at = now + Duration::seconds(lifetime_seconds);
    let id = Uuid::new_v4();
    let claims = TokenClaims {
        sub: user_id.to_string(),
        session_id,
        exp: usize::try_from(expires_at.timestamp())
            .map_err(|error| AppError::Internal(error.into()))?,
        iat: usize::try_from(now.timestamp()).map_err(|error| AppError::Internal(error.into()))?,
        jti: id.to_string(),
        iss: TOKEN_ISSUER.to_string(),
        aud: TOKEN_AUDIENCE.to_string(),
        token_use,
    };
    let value = encode(&Header::new(Algorithm::HS256), &claims, &signing_key(state))
        .map_err(|error| AppError::Internal(error.into()))?;

    Ok(IssuedToken {
        value,
        id,
        expires_at,
    })
}

fn decode_token(
    state: &AppState,
    token: &str,
    expected_use: TokenUse,
    allow_expired: bool,
) -> AppResult<TokenClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[TOKEN_ISSUER]);
    validation.set_audience(&[TOKEN_AUDIENCE]);
    validation.validate_exp = !allow_expired;

    let claims = decode::<TokenClaims>(token, &decoding_key(state), &validation)
        .map_err(|_| AppError::Unauthorized("Authentication failed".to_string()))?
        .claims;
    if claims.token_use != expected_use {
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }
    Ok(claims)
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn rate_limit_key(action: &str, subject: &str) -> String {
    let digest = Sha256::digest(subject.as_bytes());
    format!("rho-studio-fintech:rate-limit:{action}:{digest:x}")
}

fn client_ip(peer: SocketAddr, headers: &HeaderMap, trusted_proxies: &[ipnet::IpNet]) -> IpAddr {
    let peer_ip = peer.ip();
    if !trusted_proxies
        .iter()
        .any(|network| network.contains(&peer_ip))
    {
        return peer_ip;
    }

    let Some(forwarded_for) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    else {
        return peer_ip;
    };

    for candidate in forwarded_for.split(',').rev() {
        let Ok(candidate) = candidate.trim().parse::<IpAddr>() else {
            return peer_ip;
        };
        if !trusted_proxies
            .iter()
            .any(|network| network.contains(&candidate))
        {
            return candidate;
        }
    }
    peer_ip
}

async fn enforce_auth_rate_limit(
    state: &AppState,
    action: &str,
    subjects: &[String],
    limit: u32,
    config: &AuthRateLimitSection,
) -> AppResult<()> {
    const INCREMENT_WINDOW: &str = r#"
        local count = redis.call('INCR', KEYS[1])
        if count == 1 then
            redis.call('EXPIRE', KEYS[1], ARGV[1])
        end
        return {count, redis.call('TTL', KEYS[1])}
    "#;

    let mut blocked = false;
    let mut retry_after_seconds = 1_i64;
    for subject in subjects {
        let key = rate_limit_key(action, subject);
        let mut redis = state.redis.clone();
        let (count, ttl): (i64, i64) = redis::Script::new(INCREMENT_WINDOW)
            .key(key)
            .arg(config.window_seconds)
            .invoke_async(&mut redis)
            .await
            .map_err(|error| AppError::Unavailable(error.to_string()))?;
        blocked |= count > i64::from(limit);
        retry_after_seconds = retry_after_seconds.max(ttl);
    }

    if blocked {
        return Err(AppError::RateLimited {
            retry_after_seconds: u64::try_from(retry_after_seconds).unwrap_or(1),
        });
    }
    Ok(())
}

fn dummy_password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"not-a-customer-password", &salt)
            .expect("valid Argon2 parameters")
            .to_string()
    })
}

fn verify_password(password: &str, encoded_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(encoded_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

fn make_response(
    state: &AppState,
    user: UserResponse,
    session_id: Uuid,
) -> AppResult<(TokenResponse, IssuedToken)> {
    let access = issue_token(
        state,
        user.id,
        session_id,
        TokenUse::Access,
        state.config.jwt.access_expiry_seconds,
    )?;
    let refresh = issue_token(
        state,
        user.id,
        session_id,
        TokenUse::Refresh,
        state.config.jwt.refresh_expiry_seconds,
    )?;
    Ok((
        TokenResponse {
            access_token: access.value,
            refresh_token: refresh.value.clone(),
            token_type: "Bearer",
            expires_in: state.config.jwt.access_expiry_seconds,
            user,
        },
        refresh,
    ))
}

async fn insert_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    family_id: Uuid,
    refresh: &IssuedToken,
    user_agent: Option<String>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (id, user_id, token_hash, token_family_id, expires_at, user_agent)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(refresh.id)
    .bind(user_id)
    .bind(token_hash(&refresh.value))
    .bind(family_id)
    .bind(refresh.expires_at)
    .bind(user_agent)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect())
}

pub async fn signup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SignupRequest>,
) -> AppResult<Json<TokenResponse>> {
    // --- 1. Validate input shape ---------------------------------------------
    let email = request.email.trim().to_string();
    if email.is_empty()
        || email.len() > MAX_EMAIL_LENGTH
        || !email.contains('@')
        || request.password.len() < 8
        || request.password.len() > MAX_PASSWORD_LENGTH
    {
        return Err(AppError::Validation(
            "Email or password does not meet the required format".to_string(),
        ));
    }

    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(120).collect::<String>());

    // --- 2. Rate limit by IP and by email ------------------------------------
    enforce_auth_rate_limit(
        &state,
        "signup",
        &[
            format!(
                "ip:{}",
                client_ip(
                    peer,
                    &headers,
                    &state.config.auth_rate_limits.trusted_proxy_cidrs
                )
            ),
            format!("email:{}", email.to_ascii_lowercase()),
        ],
        state.config.auth_rate_limits.login_attempts,
        &state.config.auth_rate_limits,
    )
    .await?;

    // --- 3. Hash the password with Argon2 ------------------------------------
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(request.password.as_bytes(), &salt)
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error.to_string())))?  // ← fixed
        .to_string();

    // --- 4. Insert the user ---------------------------------------------------
    // NOTE: email_verified is set to true for local/dev convenience. In
    // production, set it to false and send a verification email instead.
    let insert_result = sqlx::query_as::<_, UserRecord>(
        r#"
        INSERT INTO users (email, password_hash, display_name, is_active, email_verified)
        VALUES (lower($1), $2, $3, true, true)
        RETURNING id, email, display_name, password_hash
        "#,
    )
    .bind(&email)
    .bind(&password_hash)
    .bind(display_name.as_deref())
    .fetch_one(&state.pool)
    .await;

    let user = match insert_result {
        Ok(user) => user,
        Err(sqlx::Error::Database(db_error)) if db_error.is_unique_violation() => {
            return Err(AppError::Validation(
                "Email is already registered".to_string(),
            ));
        }
        Err(error) => {
            tracing::error!(error = ?error, "signup user INSERT failed");

            return Err(AppError::Internal(error.into()));
        }
    };

    // --- 5. Auto-login: issue access + refresh tokens ------------------------
    let family_id = Uuid::new_v4();
    let user_response = UserResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
    };
    let (response, refresh) = make_response(&state, user_response, family_id)?;
    let mut transaction = state.pool.begin().await?;
    insert_refresh_token(
        &mut transaction,
        user.id,
        family_id,
        &refresh,
        user_agent(&headers),
    )
    .await?;
    transaction.commit().await?;

    Ok(Json(response))
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> AppResult<Json<TokenResponse>> {
    let email = request.email.trim();
    if email.is_empty()
        || email.len() > MAX_EMAIL_LENGTH
        || !email.contains('@')
        || request.password.is_empty()
        || request.password.len() > MAX_PASSWORD_LENGTH
    {
        return Err(AppError::Validation("Invalid credentials".to_string()));
    }
    enforce_auth_rate_limit(
        &state,
        "login",
        &[
            format!(
                "ip:{}",
                client_ip(
                    peer,
                    &headers,
                    &state.config.auth_rate_limits.trusted_proxy_cidrs
                )
            ),
            format!("email:{}", email.to_ascii_lowercase()),
        ],
        state.config.auth_rate_limits.login_attempts,
        &state.config.auth_rate_limits,
    )
    .await?;

    let user = sqlx::query_as::<_, UserRecord>(
        r#"
        SELECT id, email, display_name, password_hash
        FROM users
        WHERE lower(email) = lower($1)
          AND email_verified = true
          AND is_active = true
        "#,
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await?;

    let password_matches = match user.as_ref() {
        Some(user) => verify_password(&request.password, &user.password_hash),
        None => {
            let _ = verify_password(&request.password, dummy_password_hash());
            false
        }
    };
    let Some(user) = user.filter(|_| password_matches) else {
        return Err(AppError::Unauthorized(
            "Invalid email or password".to_string(),
        ));
    };

    let family_id = Uuid::new_v4();
    let user_response = UserResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
    };
    let (response, refresh) = make_response(&state, user_response, family_id)?;
    let mut transaction = state.pool.begin().await?;
    insert_refresh_token(
        &mut transaction,
        user.id,
        family_id,
        &refresh,
        user_agent(&headers),
    )
    .await?;
    transaction.commit().await?;

    Ok(Json(response))
}

pub async fn refresh(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<RefreshRequest>,
) -> AppResult<Json<TokenResponse>> {
    if request.refresh_token.len() > 4096 {
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }
    enforce_auth_rate_limit(
        &state,
        "refresh",
        &[
            format!(
                "ip:{}",
                client_ip(
                    peer,
                    &headers,
                    &state.config.auth_rate_limits.trusted_proxy_cidrs
                )
            ),
            format!("token:{}", token_hash(&request.refresh_token)),
        ],
        state.config.auth_rate_limits.refresh_attempts,
        &state.config.auth_rate_limits,
    )
    .await?;
    let claims = decode_token(&state, &request.refresh_token, TokenUse::Refresh, false)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("Authentication failed".to_string()))?;
    let token_id = Uuid::parse_str(&claims.jti)
        .map_err(|_| AppError::Unauthorized("Authentication failed".to_string()))?;

    let mut transaction = state.pool.begin().await?;
    let stored = sqlx::query_as::<_, StoredRefreshToken>(
        r#"
        SELECT id, user_id, token_family_id, expires_at, revoked_at
        FROM refresh_tokens
        WHERE id = $1 AND token_hash = $2
        FOR UPDATE
        "#,
    )
    .bind(token_id)
    .bind(token_hash(&request.refresh_token))
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Authentication failed".to_string()))?;

    if stored.user_id != user_id {
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }

    if stored.revoked_at.is_some() {
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, now()) WHERE token_family_id = $1",
        )
        .bind(stored.token_family_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }

    if stored.expires_at <= Utc::now() {
        sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE id = $1")
            .bind(stored.id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }

    let user = sqlx::query_as::<_, UserRecord>(
        r#"
        SELECT id, email, display_name, password_hash
        FROM users
        WHERE id = $1 AND email_verified = true AND is_active = true
        "#,
    )
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(user) = user else {
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, now()) WHERE token_family_id = $1",
        )
        .bind(stored.token_family_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    };

    let user_response = UserResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
    };
    let (response, replacement) = make_response(&state, user_response, stored.token_family_id)?;
    sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE id = $1")
        .bind(stored.id)
        .execute(&mut *transaction)
        .await?;
    insert_refresh_token(
        &mut transaction,
        user_id,
        stored.token_family_id,
        &replacement,
        user_agent(&headers),
    )
    .await?;
    transaction.commit().await?;

    Ok(Json(response))
}

pub async fn logout(
    State(state): State<AppState>,
    Json(request): Json<LogoutRequest>,
) -> StatusCode {
    if request.refresh_token.len() > 4096 {
        return StatusCode::NO_CONTENT;
    }
    if let Ok(claims) = decode_token(&state, &request.refresh_token, TokenUse::Refresh, true)
        && let (Ok(token_id), Ok(user_id)) =
            (Uuid::parse_str(&claims.jti), Uuid::parse_str(&claims.sub))
    {
        let _ = sqlx::query(
            r#"
            UPDATE refresh_tokens
            SET revoked_at = COALESCE(revoked_at, now())
            WHERE token_family_id = (
                SELECT token_family_id
                FROM refresh_tokens
                WHERE id = $1 AND user_id = $2 AND token_hash = $3
            )
            "#,
        )
        .bind(token_id)
        .bind(user_id)
        .bind(token_hash(&request.refresh_token))
        .execute(&state.pool)
        .await;
    }
    StatusCode::NO_CONTENT
}

pub(super) fn bearer_token(headers: &HeaderMap) -> AppResult<&str> {
    let value = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Authentication failed".to_string()))?;
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| AppError::Unauthorized("Authentication failed".to_string()))
}

pub(super) async fn authenticated_user_id(state: &AppState, token: &str) -> AppResult<Uuid> {
    let claims = decode_token(state, token, TokenUse::Access, false)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("Authentication failed".to_string()))?;
    let session_active: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM refresh_tokens
            WHERE token_family_id = $1
              AND revoked_at IS NULL
              AND expires_at > now()
        )
        "#,
    )
    .bind(claims.session_id)
    .fetch_one(&state.pool)
    .await?;
    if !session_active {
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }
    Ok(user_id)
}

pub async fn logout_all(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    let user_id = authenticated_user_id(&state, bearer_token(&headers)?).await?;
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, now()) WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<UserResponse>> {
    let user_id = authenticated_user_id(&state, bearer_token(&headers)?).await?;
    let user = sqlx::query_as::<_, UserResponse>(
        r#"
        SELECT id, email, display_name
        FROM users
        WHERE id = $1 AND email_verified = true AND is_active = true
        "#,
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Authentication failed".to_string()))?;
    Ok(Json(user))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_hash_is_deterministic_and_not_the_token() {
        let token = "opaque-example-refresh-token";
        let hash = token_hash(token);
        assert_eq!(hash, token_hash(token));
        assert_ne!(hash, token);
    }

    #[test]
    fn rate_limit_keys_do_not_expose_subject_values() {
        let key = rate_limit_key("login", "email:customer@example.test");
        assert!(key.starts_with("rho-studio-fintech:rate-limit:login:"));
        assert!(!key.contains("customer@example.test"));
        assert_eq!(key, rate_limit_key("login", "email:customer@example.test"));
    }

    #[test]
    fn forwarded_client_ip_is_used_only_for_trusted_proxy_peers() {
        let trusted = vec!["10.0.0.0/8".parse().expect("valid test CIDR")];
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 10.0.0.2".parse().expect("valid header"),
        );

        assert_eq!(
            client_ip(
                "10.0.0.10:443".parse().expect("valid peer"),
                &headers,
                &trusted
            ),
            "203.0.113.7".parse::<IpAddr>().expect("valid IP")
        );
        assert_eq!(
            client_ip(
                "198.51.100.4:54321".parse().expect("valid peer"),
                &headers,
                &trusted
            ),
            "198.51.100.4".parse::<IpAddr>().expect("valid IP")
        );
    }

    #[test]
    fn password_verification_accepts_only_matching_password() {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"correct-horse-battery-staple", &salt)
            .expect("hash password")
            .to_string();

        assert!(verify_password("correct-horse-battery-staple", &hash));
        assert!(!verify_password("incorrect-password", &hash));
    }
}

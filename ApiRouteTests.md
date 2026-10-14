# API Route Test Strategy

**Test sources:** `src/route_tests.rs`, `src/route_tests/database.rs`, plus `#[cfg(test)]` modules in `src/domain/consent.rs`, `src/config.rs`, `src/error.rs`, `src/routes/auth.rs`
**Router source:** `src/main.rs` (`build_app`)
**Current result:** 22 tests passed, 0 failed, 0 ignored. Wall time ~11.8 s.

## Overview

The test suite exercises the same Axum router used by the production server. It is organized in two tiers, each with a different cost/coverage profile.

| Tier | Location | Infrastructure | Runtime | What it proves |
| --- | --- | --- | --- | --- |
| Tier 1 — Router boundary | `src/route_tests.rs` | Mock Redis, lazy PgPool, `TestStorage` | Milliseconds | Validation, routing, extractor behavior, auth gating |
| Tier 2 — Container backed | `src/route_tests/database.rs` | Real PostgreSQL 16 via `testcontainers`, mock Redis, `TestStorage` | ~10 s total | Persistence, uniqueness, transactions, session lifecycle, consent lifecycle |
| Unit | `#[cfg(test)]` inside modules | None | Milliseconds | Pure logic (domain transitions, config validation, error mapping, password verification, IP resolution) |

Tier 2 requires Docker. Tier 1 does not.

## Unit Tests

Pure logic tests that do not touch the router or any external dependency.

| Test | Source | Asserts |
| --- | --- | --- |
| `domain::consent::tests::grants_require_publication_and_repeated_grants_are_idempotent` | `src/domain/consent.rs` | A grant on a published notice records an event; a repeat grant is a no-op |
| `domain::consent::tests::withdrawal_requires_a_grant_and_repeated_withdrawals_are_idempotent` | `src/domain/consent.rs` | Withdrawal requires a prior grant; a repeat withdrawal is a no-op |
| `domain::consent::tests::database_event_types_round_trip` | `src/domain/consent.rs` | `ConsentAction` ↔ database string round trip |
| `config::tests::cors_origins_must_be_explicit_and_production_origins_must_use_https` | `src/config.rs` | Production CORS rejects wildcards and non-HTTPS origins |
| `error::tests::rate_limit_response_includes_retry_after` | `src/error.rs` | `AppError::RateLimited` renders `429` with a `Retry-After` header |
| `routes::auth::tests::refresh_hash_is_deterministic_and_not_the_token` | `src/routes/auth.rs` | `token_hash` is deterministic and never equals the input |
| `routes::auth::tests::rate_limit_keys_do_not_expose_subject_values` | `src/routes/auth.rs` | Rate-limit keys are namespaced and do not contain raw subject values |
| `routes::auth::tests::forwarded_client_ip_is_used_only_for_trusted_proxy_peers` | `src/routes/auth.rs` | `X-Forwarded-For` honored only from configured trusted proxies |
| `routes::auth::tests::password_verification_accepts_only_matching_password` | `src/routes/auth.rs` | Argon2 verify accepts the correct password and rejects the wrong one |

## Tier 1 — Router Boundary Tests

Boots the full router via `build_app` with the following substitutes:

- **PgPool** — lazy, points at `postgres://test:test@127.0.0.1:1/rho_test`. The port `1` is intentional: the pool never connects. Any query returns `PoolTimedOut` after the acquire timeout. Boundary tests must fail before touching the database, not after.
- **Redis** — an in-process RESP server bound to an ephemeral port. Responds to `PING` with `+PONG`, to `EVAL` and `EVALSHA` with a two-element integer array `*2\r\n:1\r\n:60\r\n` (the shape the rate limiter's Lua script returns), and to any other command with `+OK`.
- **ObjectStorage** — `TestStorage`. No-op put and delete, empty get, deterministic presigned URL prefix `http://storage.test/{key}`.

| Method and path | Test | Assertion |
| --- | --- | --- |
| `GET /health` | `get_health_returns_liveness` | `200`; body contains `status: "ok"` and `service: "rho-studio-fintech"` |
| `GET /health/ready` | `get_health_ready_returns_dependency_checks` | `200`; `checks.database`, `checks.redis`, `checks.storage` are booleans. The test does not require all dependencies to be healthy. |
| `GET /version` | `get_version_returns_package_identity` | `200`; `name` equals `CARGO_PKG_NAME` |
| `POST /api/v1/auth/login` | `post_auth_login_rejects_invalid_credentials_shape` | Empty email returns `422` before any database lookup |
| `POST /api/v1/auth/refresh` | `post_auth_refresh_rejects_malformed_json` | Syntactically invalid JSON returns Axum `400` |
| `POST /api/v1/auth/logout` | `post_auth_logout_rejects_malformed_json` | Missing required `refresh_token` field returns `422` |
| `POST /api/v1/auth/logout-all` | `post_auth_logout_all_requires_authentication` | Missing bearer token returns `401` |
| `GET /api/v1/me` | `get_me_requires_authentication` | Missing bearer token returns `401` |
| `GET /api/v1/consents/notices?locale=es-MX` | `get_consent_notices_requires_authentication` | Missing bearer token returns `401` |
| `GET /api/v1/me/consents` | `get_my_consents_requires_authentication` | Missing bearer token returns `401` |
| `POST /api/v1/consents` | `post_consent_requires_authentication` | A valid-shaped grant without a bearer token returns `401` |

### Why `ConnectInfo` is injected

Endpoints that extract `ConnectInfo<SocketAddr>` — `signup`, `login`, `refresh` — need the extension to be present on the request. The production server gets it from the TCP listener via `into_make_service_with_connect_info::<SocketAddr>()`. `tower::ServiceExt::oneshot` bypasses the listener, so the request helpers insert a synthetic `ConnectInfo` when `include_connect_info` is `true`. Without it, the extractor rejects and the request returns `500`, which would mask the behavior the test is trying to verify.

## Tier 2 — Container-Backed Tests

Each test in `src/route_tests/database.rs` starts a fresh PostgreSQL 16 container, applies the full migration set via `db::run_migrations`, seeds the data it needs, and drives the production router through the full request/response cycle.

### Test environment

`TestEnv` owns the container handle and drops it when the test function returns. The `Drop` impl on `TestApp` (in the parent module) aborts the mock Redis task.

> **Gotcha.** The pattern `let TestEnv { app, pool, _postgres } = start_test_env().await;` binds `_postgres` as a real field so the container stays alive. Using `..` instead would drop the container immediately and every query would fail with `UnexpectedEof` or `PoolTimedOut`. This was a real debugging incident in the earlier phase; the header on `database.rs` documents it.

### Test 1 — Signup

`postgres_backed_signup_creates_user_and_rejects_duplicate`

| Step | Assertion |
| --- | --- |
| `POST /auth/signup` with a UUID-suffixed email | `200 OK` with a valid `access_token` and `refresh_token`; `token_type = "Bearer"` |
| Response `user` block | `email` matches the submitted value; `display_name` matches |
| Database `users` row | Exists; `password_hash` starts with `$argon2`; is not the plaintext password |
| Database `users` row | `email_verified = true` (dev shortcut); `is_active = true` |
| Database `refresh_tokens` | Exactly one row for the new user (the auto-login session) |
| Duplicate signup with the same email | `422 Unprocessable Entity` |
| `users` row count for that email | Still 1 — no second row created |

### Test 2 — Full auth and consent flow

`postgres_backed_auth_refresh_logout_profile_and_consent_flow`

Seeds one active, verified user (`route-test@example.test`) with an Argon2id hash, plus one published notice for `privacy / es-MX / test-v1`. Then exercises:

| Step | Assertion |
| --- | --- |
| `POST /auth/login` with mixed-case email (`ROUTE-TEST@`) | `200 OK`; login is case-insensitive |
| Response `user` block | `id` matches the seeded user; `email` is the lowercased form |
| Database lookup of refresh token by `token_hash` | Row exists; the raw token is never stored |
| `GET /me` with the access token | `200 OK`; `id` and `display_name` match the seeded user |
| `GET /consents/notices?locale=es-MX` | `200 OK`; exactly one notice; `id` matches the seeded notice |
| `GET /me/consents` (initial state) | `200 OK`; empty array — no consent events yet |
| `POST /consents` with `action=granted` | `204 No Content`; event count becomes 1 |
| Repeat `granted` | `204 No Content`; event count stays 1 — idempotent |
| `POST /consents` with `action=withdrawn` | `204 No Content`; event count becomes 2 |
| Repeat `withdrawn` | `204 No Content`; event count stays 2 — idempotent |
| `GET /me/consents` after withdrawal | `action = "withdrawn"`, `version = "test-v1"` |
| `POST /auth/refresh` with the current refresh token | `200 OK`; new access and refresh tokens; new refresh differs from old |
| Database lookup of the old token | `revoked_at IS NOT NULL` — rotation revoked it |
| Replay `POST /auth/refresh` with the **old** refresh token | `401 Unauthorized` |
| Count of active tokens in the family | `0` — replay revoked the entire family |
| `GET /me` with the rotated access token | `401 Unauthorized` — the family is revoked, so the access token is rejected even though it hasn't expired |
| Second `POST /auth/login` | `200 OK`; new token family |
| `POST /auth/logout` with the second refresh token | `204 No Content` |
| Count of active tokens in the second family | `0` — logout revoked it |
| `GET /me` with the second session's access token | `401 Unauthorized` |

This test is the security-critical assertion set for the identity layer. It proves rotation, replay detection, family-wide revocation, session-liveness enforcement, and the interaction between `/logout` and outstanding access tokens.

### What the container tests do not cover

- **Rate limiting** uses the mock RESP server. Real Redis limits, expiry, and outage behavior are not exercised.
- **Object storage** uses `TestStorage`. Real S3 upload, presigned URL issuance, and access auditing are not exercised.
- **Concurrent requests** are not part of the test. Refresh replay and idempotent signup are tested sequentially.

## Running the Suite

Run every test (requires Docker for Testcontainers):

```powershell
cargo test --locked
```

Run only Tier 1 (no Docker required):

```powershell
cargo test route_tests:: -- --skip database
```

Run only the container-backed signup test:

```powershell
cargo test route_tests::database::postgres_backed_signup_creates_user_and_rejects_duplicate -- --nocapture
```

Run only the full auth + consent flow test:

```powershell
cargo test route_tests::database::postgres_backed_auth_refresh_logout_profile_and_consent_flow -- --nocapture
```

Run all container-backed tests with single-threaded execution (recommended on resource-constrained machines):

```powershell
cargo test route_tests::database:: -- --test-threads=1 --nocapture
```

Run only the unit tests:

```powershell
cargo test --lib -- --skip route_tests
```

## Expected Output

```text
running 22 tests
test domain::consent::tests::grants_require_publication_and_repeated_grants_are_idempotent ... ok
test domain::consent::tests::withdrawal_requires_a_grant_and_repeated_withdrawals_are_idempotent ... ok
test domain::consent::tests::database_event_types_round_trip ... ok
test config::tests::cors_origins_must_be_explicit_and_production_origins_must_use_https ... ok
test error::tests::rate_limit_response_includes_retry_after ... ok
test route_tests::get_consent_notices_requires_authentication ... ok
test route_tests::get_health_returns_liveness ... ok
test route_tests::get_me_requires_authentication ... ok
test route_tests::get_my_consents_requires_authentication ... ok
test route_tests::get_version_returns_package_identity ... ok
test route_tests::post_auth_login_rejects_invalid_credentials_shape ... ok
test route_tests::post_auth_logout_all_requires_authentication ... ok
test route_tests::post_auth_logout_rejects_malformed_json ... ok
test route_tests::post_auth_refresh_rejects_malformed_json ... ok
test route_tests::post_consent_requires_authentication ... ok
test routes::auth::tests::forwarded_client_ip_is_used_only_for_trusted_proxy_peers ... ok
test route_tests::get_health_ready_returns_dependency_checks ... ok
test routes::auth::tests::rate_limit_keys_do_not_expose_subject_values ... ok
test routes::auth::tests::refresh_hash_is_deterministic_and_not_the_token ... ok
test routes::auth::tests::password_verification_accepts_only_matching_password ... ok
test route_tests::database::postgres_backed_signup_creates_user_and_rejects_duplicate ... ok
test route_tests::database::postgres_backed_auth_refresh_logout_profile_and_consent_flow ... ok

test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.80s
```

## Coverage Summary

| Category | Tests | Backing |
| --- | --- | --- |
| Domain (consent transitions) | 3 | Pure unit |
| Config validation | 1 | Pure unit |
| Error model | 1 | Pure unit |
| Auth module helpers | 4 | Pure unit |
| Route boundaries (health, auth, consent) | 10 | `route_tests.rs` (mock Redis, lazy pool) |
| Auth + consent end-to-end | 2 | `route_tests/database.rs` (testcontainers) |
| **Total** | **22** | |

## Remaining Test Gaps

The following are not yet covered by the suite. Each is tracked against a Feature in [DevPlan.md](DevPlan.md).

### Identity (Feature F2)

- **Real Redis rate limiting.** The mock Redis returns a canned response for `EVAL` and `EVALSHA`. Real window expiry, `Retry-After` semantics, and Redis outage behavior (fail closed with `503`) are not covered.
- **Concurrent refresh replay.** Replay detection is tested sequentially. Two concurrent requests with the same refresh token should race such that exactly one wins and one gets `401`; this is not asserted.
- **Successful `logout-all` persistence.** `POST /auth/logout-all` is only tested at the auth-gating boundary (`401` without a token). The full revocation of every family for the user is not asserted end-to-end.
- **Email verification flow.** Not implemented. When it is, it needs positive and negative tests.
- **Password reset flow.** Not implemented. When it is, it needs positive and negative tests.
- **Password strength policy.** Not implemented. When it is, it needs a table of accepted and rejected passwords.
- **`kid`-based JWT signing key rotation.** Not implemented. When it is, it needs tests that old and new keys both validate during the rotation window.
- **Session listing and per-session revocation.** Not implemented.

### Onboarding and KYC (Feature F3)

- **Onboarding route registration.** Handlers exist but are not registered in `build_app`. No test can exercise them.
- **KYC provider sandbox integration.** Not implemented.
- **Document upload and quarantine.** Not implemented.
- **Customer ↔ user linkage.** The `customers.user_id` column exists but no handler populates it. When the linkage ships, it needs positive, negative, and idempotency tests.

### Consent

- **Real `POST /consents` rejection paths.** The boundary test asserts `401` on an unauthenticated request. Rejection of a grant against a non-published notice, a duplicate grant, a withdrawal without a prior grant, and a version mismatch are exercised by unit tests in `src/domain/consent.rs` but not through the HTTP layer.
- **Notice content hash verification on read.** The handler recomputes the SHA-256 of every published notice it returns. A test that seeds a mismatch and asserts a `500` is not present.
- **Notice retirement and re-publication.** Not covered.
- **Multi-locale notice listing.** Only `es-MX` is tested.

### Financial Workflows (Features F4 through F9)

No account, loan, payment, repayment, or transfer routes are registered. Everything in [banking-domain.md](banking-domain.md) is untested at the HTTP layer because it does not exist at the HTTP layer.

### Cross-Cutting

- **Readiness semantics.** The boundary test asserts the response shape (three boolean fields). It does not assert that a specific dependency being down flips its boolean to `false`. Real Redis and S3 dependency failure are not tested.
- **CORS preflight.** No test covers `OPTIONS` requests or the `Access-Control-*` response headers.
- **Request body size limits.** The 20 MB global limit is not exercised.
- **Content-Type enforcement.** Requests with `Content-Type: application/xml` are not tested against a JSON endpoint.
- **Structured error code registry.** The `code` field in the error body is asserted indirectly through status codes, not by asserting the exact code string.
- **PII redaction.** No test asserts that a request containing an email or CLABE does not appear in `tracing` output.

## Test File Conventions

Both test files carry a standardized header modeled on the migration headers:

- Banner, author, and date
- A `Description:` block covering purpose, dependencies, design notes, and known gaps

`src/route_tests.rs` documents Tier 1.
`src/route_tests/database.rs` documents Tier 2.

When adding tests, follow the existing pattern:

1. Use the appropriate `test_app()` or `test_app_with_pool(pool, url)` helper.
2. Use `request_with_headers` when the endpoint needs a bearer token or a specific header.
3. Set `include_connect_info: true` only for endpoints that extract `ConnectInfo<SocketAddr>`.
4. Prefer UUID-suffixed emails and other unique fixtures so tests are rerunnable.
5. Clean up test data or rely on the ephemeral container; do not leave residue in a shared database.

---
**Rho.Studio — Engineering Department**
Contact: [alexis.tercero@rho.studio](mailto:alexis.tercero@rho.studio)
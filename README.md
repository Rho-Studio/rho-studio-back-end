# Rho Studio Fintech Backend

Rust/Axum backend foundation for a Mexico-first mobile lending product. The
running service provides health/version checks, self-service signup and
session authentication, and consent notice/evidence routes. Customer accounts,
loans, transfers, repayments, ledger posting, and live money movement are not
implemented.

Use these docs according to their scope:

- [Architecture](architecture.md): current runtime boundaries and target shape.
- [Data model](DataModel.md): schema entities, relationships, and Rho-colored ER diagram.
- [Full data model diagram](datamaodel.mmd): all current entities and relationships.
- [Banking domain](banking-domain.md): account, loan, and payment schema,
  invariants, gaps, and proposed API resources.
- [Development plan](DevPlan.md): detailed target architecture, backlog, and
  release gates. Planned features are not live features.
- [Mobile client contract](mobile-backend.md): current auth/consent behavior
  and future financial-client constraints.
- [Network boundaries](deployment-network.md): local Compose bindings and
  production network expectations.
- [API route tests](ApiRouteTests.md): route-by-route automated test coverage
  and current gaps.
- [Progress report](ProgressReport.md): verified implementation status and
  remaining priorities.

## Requirements

- Docker Desktop with Compose v2 for the local dependency stack.
- Rust 1.88 or newer to build and test locally.

## Local Setup

Create a local environment file from the template and change secrets for your
own development environment:

```powershell
Copy-Item .env.example .env
```

The checked-in example credentials are local-only. Do not use them outside a
disposable development environment, and do not commit `.env`.

Start the API and its local dependencies:

```powershell
docker compose up -d --build
docker compose ps
```

The backend waits for PostgreSQL, Redis, and RustFS to report healthy before
starting. SQLx migrations run automatically during startup.

Check liveness and readiness:

```powershell
curl.exe http://127.0.0.1:8080/health
curl.exe http://127.0.0.1:8080/health/ready
```

The backend waits for PostgreSQL, Redis, and RustFS to report healthy. The
readiness endpoint checks PostgreSQL, Redis, and the configured S3 bucket.
Compose runs SQLx migrations during backend startup.

Stop services without deleting local data:

```powershell
docker compose down
```

`docker compose down -v` deletes the local PostgreSQL and RustFS volumes. It is
destructive and is not a data-retirement or backup procedure.

## Implemented API

All listed routes are registered in `src/main.rs`.

| Method | Path | Behavior |
| --- | --- | --- |
| `GET` | `/health` | Process liveness |
| `GET` | `/health/ready` | PostgreSQL, Redis, and S3 bucket readiness |
| `GET` | `/version` | Package version and optional Git SHA |
| `POST` | `/api/v1/auth/signup` | Register a new user and issue a token pair |
| `POST` | `/api/v1/auth/login` | Sign in an active, email-verified user |
| `POST` | `/api/v1/auth/refresh` | Rotate a refresh token |
| `POST` | `/api/v1/auth/logout` | Revoke the supplied token family |
| `POST` | `/api/v1/auth/logout-all` | Revoke all sessions for the authenticated user |
| `GET` | `/api/v1/me` | Return the authenticated user's profile projection |
| `GET` | `/api/v1/consents/notices?locale=es-MX` | List published notices for a locale |
| `GET` | `/api/v1/me/consents` | Return the authenticated user's latest consent states |
| `POST` | `/api/v1/consents` | Record a grant or withdrawal for a notice version |

Protected routes require ``Authorization: Bearer <access_token>``. Self-service signup is implemented; email verification and password recovery are not. Signup currently creates users with email_verified = true for development convenience — the production flow must add a verification step before that column can meaningfully gate login. Account provisioning, KYC, and all financial workflows remain unimplemented.

Consent notices are not seeded or auto-published. The notices route can return
an empty list, and a grant cannot be recorded until an approved notice is
published operationally. Consent evidence recorded through this API is not a legal determination that consent is the applicable lawful basis for any processing activity.

There are no account, loan-application, loan, payment, repayment, or transfer
routes. Existing financial migrations are schema groundwork only. See
[Banking domain](banking-domain.md) before implementing those workflows.

## Local Services

| Service | Host access | Compose address |
| --- | --- | --- |
| Backend | `http://127.0.0.1:8080` (published on all host interfaces) | `backend:8080` |
| PostgreSQL | `127.0.0.1:5432` | `postgres:5432` |
| Redis | `127.0.0.1:6379` | `redis:6379` |
| RustFS S3 API | `127.0.0.1:9000` | `rustfs:9000` |
| RustFS console | `http://127.0.0.1:9001` | `rustfs:9001` |

Only the backend is bound to all host interfaces, so a mobile device on the
same LAN can reach it during development. Every other service binds to host
loopback (127.0.0.1), which keeps the database, cache, and object store off
the LAN. The loopback bindings are useful for direct inspection: 

´´´powershell
docker compose exec redis redis-cli KEYS "rho-studio-fintech:rate-limit:*"
docker compose exec rustfs sh -c "ls -la /data"
Start-Process "http://127.0.0.1:9001"
´´´
Production must not mirror these bindings. See [Network boundaries](deployment-network.md) for the target topology.

## Configuration

The application reads process environment variables and loads `.env` for local
development. Defaults below come from `src/config.rs`.

| Variable | Required | Description |
| --- | --- | --- |
| `APP_ENV` | No | Runtime environment; defaults to `development` |
| `APP_HOST` / `APP_PORT` | No | Bind address; defaults to `0.0.0.0:8080` |
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `DATABASE_MAX_CONNECTIONS` | No | Pool limit; defaults to `20` |
| `REDIS_URL` | Yes | Redis connection string |
| `JWT_SECRET` | Yes | HS256 signing secret; production requires at least 32 bytes |
| `JWT_EXPIRY_SECONDS` | No | Access-token lifetime; defaults to `3600`, maximum `3600` |
| `JWT_REFRESH_EXPIRY_SECONDS` | No | Refresh lifetime; defaults to `2592000`, maximum `7776000` |
| `CORS_ALLOWED_ORIGINS` | Production | Explicit comma-separated origins; production requires HTTPS |
| `AUTH_RATE_LIMIT_WINDOW_SECONDS` | No | Login/refresh rate-limit window; defaults to `900` |
| `AUTH_LOGIN_RATE_LIMIT_ATTEMPTS` | No | Login limit per window; defaults to `10` |
| `AUTH_REFRESH_RATE_LIMIT_ATTEMPTS` | No | Refresh limit per window; defaults to `30` |
| `TRUSTED_PROXY_CIDRS` | Production | Trusted proxy ranges for client-IP resolution |
| `STORAGE_BACKEND` | No | Log label; the current implementation uses S3-compatible storage |
| `STORAGE_AUTO_CREATE_BUCKET` | No | Development bucket bootstrap; defaults to `false` |
| `S3_ENDPOINT` | Yes | S3-compatible API endpoint |
| `S3_ACCESS_KEY_ID` | Yes | Storage access key |
| `S3_SECRET_ACCESS_KEY` | Yes | Storage secret |
| `S3_BUCKET` | Yes | Private bucket name for this environment |
| `S3_REGION` | No | Signing region; defaults to `us-east-1` |
| `RUST_LOG` | No | Tracing filter; defaults to `info` |

### Environment-specific requirements:

- **dev**: demo credentials are acceptable; ``.env`` is gitignored.
- **test, prod**: secrets come from a secret manager, not .``env`` or source.
``DATABASE_URL`` must include ``sslmode=require``. ``CORS_ALLOWED_ORIGINS`` and
``TRUSTED_PROXY_CIDRS`` are mandatory.

Forwarding headers from untrusted peers are ignored. Authentication rate
limiting returns ``429`` with ``Retry-After`` and fails closed with ``503`` if Redis
is unavailable.

## Development Checks

```powershell
cargo check
cargo test --locked
cargo build --locked
```

The test suite includes a Testcontainers-backed PostgreSQL integration test;
keep Docker running when invoking `cargo test --locked`. See
[API route tests](ApiRouteTests.md) for the covered flows and remaining gaps.

Validate the database migrations and data-model constraints in an isolated
temporary database:

```powershell
.\scripts\verify-data-model.ps1
```

The script cleans only its dedicated validation database and runner; it does
not delete the normal development database or volumes.

Run formatting checks before submitting changes:

```powershell
cargo fmt --all -- --check
```

## Further Reading

- [Architecture overview](architecture.md)
- [Data model](DataModel.md)
- [Banking domain](banking-domain.md)
- [Development plan](DevPlan.md)
- [Mobile client contract](mobile-backend.md)
- [Network boundaries](deployment-network.md)

---
**[Rho.Studio®](https://rho.studio/) - Engineering Department** - Contact [alexis.tercero@rho.studio](mailto:alexis.tercero@rho.studio)
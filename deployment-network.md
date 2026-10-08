# Deployment Network Boundaries

Bindings, addresses, and exposure per environment: **dev**, **test**, **prod**.

---

## dev

| Service | Host binding | Reachable from |
|---------|-------------|----------------|
| backend | `0.0.0.0:8080` | LAN + host |
| postgres | `127.0.0.1:5432` | host |
| redis | `127.0.0.1:6379` | host |
| rustfs (S3 API) | `127.0.0.1:9000` | host |
| rustfs (console) | `127.0.0.1:9001` | host |

**Address rules**

| Caller | Target | Address |
|--------|--------|---------|
| host (`cargo run`) | postgres | `127.0.0.1:5432` |
| host | redis | `127.0.0.1:6379` |
| host | rustfs | `127.0.0.1:9000` |
| container | postgres | `postgres:5432` |
| container | redis | `redis:6379` |
| container | rustfs | `rustfs:9000` |

**Config**

| Variable | Host | Container |
|----------|------|-----------|
| `APP_ENV` | `development` | `development` |
| `APP_HOST` | `0.0.0.0` | `0.0.0.0` |
| `APP_PORT` | `8080` | `8080` |
| `DATABASE_URL` | `...@127.0.0.1:5432/...` | `...@postgres:5432/...` |
| `REDIS_URL` | `redis://127.0.0.1:6379/` | `redis://redis:6379` |
| `S3_ENDPOINT` | `http://127.0.0.1:9000` | `http://rustfs:9000` |
| `TRUSTED_PROXY_CIDRS` | empty | empty |

**Rules**

- `localhost` never appears in a committed file. Use `127.0.0.1`.
- `0.0.0.0` only on the backend bind.
- Demo credentials allowed; `.env` gitignored.
- LAN clients reach the backend only. Data services stay loopback.

**Verify**

```powershell
docker compose port backend 8080    # 0.0.0.0:8080
docker compose port postgres 5432   # 127.0.0.1:5432
docker compose port redis 6379      # 127.0.0.1:6379
docker compose port rustfs 9000     # 127.0.0.1:9000
docker compose port rustfs 9001     # 127.0.0.1:9001
```

## test

| Service | Public | Private |
|---------|--------|---------|
| backend | `https://api-staging.example.com` | pod `8080` |
| postgres | — | private endpoint |
| redis | — | private endpoint |
| object storage | — | private bucket |

**Config**

| Variable | Value |
|----------|-------|
| `APP_ENV` | `staging` |
| `APP_HOST` / `APP_PORT` | `0.0.0.0` / `8080` |
| `DATABASE_URL` | managed, `sslmode=require` |
| `REDIS_URL` | managed, TLS |
| `JWT_SECRET` | secret manager |
| `TRUSTED_PROXY_CIDRS` | load-balancer CIDRs |
| `CORS_ALLOWED_ORIGINS` | explicit HTTPS origins |

**Forbidden**

- Production data or credentials.
- Host-published data services.
- Wildcards in CORS or `sslmode`.

---

## prod

**Public surface**

| Path | Auth |
|------|------|
| `/health`, `/health/ready`, `/version` | none |
| `/api/v1/*` | bearer |
| everything else | 404 |

**Private**

postgres, redis, object storage, queue, secret manager, metrics, admin tools. None bind to a host port.

**Config**

| Variable | Value |
|----------|-------|
| `APP_ENV` | `production` |
| `APP_HOST` / `APP_PORT` | `0.0.0.0` / `8080` |
| `DATABASE_URL` | managed, `sslmode=require`, secret manager |
| `REDIS_URL` | managed, TLS, secret manager |
| `JWT_SECRET` | secret manager, at least 32 bytes, rotated |
| `JWT_EXPIRY_SECONDS` | at most 3600 |
| `JWT_REFRESH_EXPIRY_SECONDS` | at most 2592000 |
| `CORS_ALLOWED_ORIGINS` | explicit HTTPS, no wildcards |
| `TRUSTED_PROXY_CIDRS` | load-balancer CIDRs, never empty |
| `RUST_LOG` | `info` or higher |

**Required at the edge**

TLS 1.2+, WAF, DDoS protection, rate limiting, explicit CORS, per-route body limits, trusted-proxy parsing.

**Forbidden**

- Any data service publicly reachable.
- `localhost` in config.
- Wildcard CORS.
- Demo credentials.
- Secrets in source, `.env`, or images.
- Swagger UI publicly reachable.
- `sslmode` weaker than `require`.
- Empty `TRUSTED_PROXY_CIDRS` behind a proxy.
- `RUST_LOG=debug`.

**Reverse proxy**

`TRUSTED_PROXY_CIDRS` must be the exact proxy ranges. `X-Forwarded-For` is honored only from trusted peers, parsed right-to-left. It informs rate limiting only, never authentication.

---

## Cross-environment rules

| Rule | dev | test | prod |
|------|-----|------|------|
| `localhost` in committed config | No | No | No |
| Service names for container-to-container | Yes | Yes | Yes |
| `127.0.0.1` for host-to-published-service | Yes | — | — |
| Secrets from a manager | — | Yes | Yes |
| TLS on database connection | — | Yes | Yes |
| Explicit CORS origins | — | Yes | Yes |
| Data services bind to host ports | Yes | No | No |

---

## Pre-release checklist

- [ ] Firewall verified from outside the app host
- [ ] Data services unreachable from public networks
- [ ] Backend container bindings inspected directly
- [ ] `TRUSTED_PROXY_CIDRS` matches actual proxy range
- [ ] `CORS_ALLOWED_ORIGINS` has no wildcards
- [ ] `JWT_SECRET` from manager, length verified
- [ ] `DATABASE_URL` includes `sslmode=require`
- [ ] `/health/ready` returns 200 through the edge
- [ ] Swagger UI returns 404 publicly
- [ ] Every ingress path has an owner and review date

Any unchecked item is a release blocker.

---
**[Rho.Studio®](https://rho.studio/) - Engineering Department** - Contact [alexis.tercero@rho.studio](mailto:alexis.tercero@rho.studio)
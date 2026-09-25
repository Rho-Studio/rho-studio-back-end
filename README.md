# Rho Studio Backend

Rust backend for the Rho Studio workout routines iOS app.

## Local Development

Requirements:

- Docker and Docker Compose
- Rust 1.88 or newer
- `curl`
- `jq`

Start Postgres, Redis, RustFS, and the backend:

```bash
docker compose up -d --build
```

Import the exercise dataset:

```bash
./scripts/import-exercises.sh
```

The API is available at:

```text
http://127.0.0.1:8080
```

## Exercise Catalog

Request all published exercises:

```bash
curl http://127.0.0.1:8080/api/v1/exercises | jq .
```

Run the catalog smoke test:

```bash
./scripts/test-exercises-endpoint.sh
```

The response is a JSON array. Each exercise has a short public dataset ID:

```json
{
  "id": "0001",
  "name": "3/4 sit-up",
  "description": "English exercise instructions",
  "gif_url": "http://localhost:9000/fitness-gifs/videos/0001-2gPfomN.gif",
  "muscle_group": "waist",
  "equipment": "body weight",
  "category": "waist"
}
```

The local dataset contains 1,324 exercises. GIF URLs use the local RustFS
service on port `9000`.

The development configuration creates the `fitness-gifs` bucket on startup.
Production deployments must provision the bucket through infrastructure
automation and leave `STORAGE_AUTO_CREATE_BUCKET=false`.

## Routine Endpoint

Development demo request:

```bash
curl http://127.0.0.1:8080/api/v1/routines/demo | jq .
```

Routine by UUID:

```bash
curl http://127.0.0.1:8080/api/v1/routines/<routine-id> | jq .
```

Run the routine and catalog integration tests:

```bash
./scripts/test-routine-endpoint.sh
./scripts/test-exercises-endpoint.sh
```

## Local Services

| Service | URL or port |
| --- | --- |
| Backend | `http://127.0.0.1:8080` |
| Exercise API | `http://127.0.0.1:8080/api/v1/exercises` |
| Postgres | `localhost:5432` |
| Redis | `localhost:6379` |
| RustFS S3 API | `http://localhost:9000` |
| RustFS console | `http://localhost:9001` |

Stop the services:

```bash
docker compose down
```

## Validation

```bash
cargo fmt --all -- --check
cargo test --locked
cargo build --locked
```

The project includes a GitHub Actions workflow for automated formatting and
test validation.

## Documentation

- [Development plan](devPlan.md)
- [Local walkthrough](walktrough.md)

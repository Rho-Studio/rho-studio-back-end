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
 * File:        src/db.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-07
 * ============================================================================
 * Description:
 *      PostgreSQL pool initialization and migration runner for
 *      rho-studio-fintech. Executed once during application startup in
 *      `main.rs`; the resulting `PgPool` is cloned into `AppState` and
 *      used by every handler that touches the database.
 *
 *      Functions:
 *          - init_pool          Creates the shared connection pool with
 *                               bounded size, acquire timeout, idle
 *                               timeout, and max lifetime. Fails fast if
 *                               the database is unreachable.
 *          - run_migrations     Applies all migrations in ./migrations in
 *                               order via `sqlx::migrate!`. Idempotent —
 *                               migrations already recorded in
 *                               `_sqlx_migrations` are skipped.
 *
 *      Pool parameters:
 *          - max_connections    Configured via `DatabaseSection::max_connections`.
 *          - min_connections    2 — keeps a warm floor for low-latency
 *                               first requests after idle periods.
 *          - acquire_timeout    5s — a request that cannot get a connection
 *                               within 5 seconds fails with PoolTimedOut
 *                               rather than hanging indefinitely.
 *          - idle_timeout       600s (10 min) — idle connections are
 *                               closed to free server-side resources.
 *          - max_lifetime       1800s (30 min) — periodic recycling
 *                               prevents long-lived connections from
 *                               accumulating state after a database
 *                               failover or config change.
 *
 *      Migrations:
 *          `sqlx::migrate!` embeds the migration files into the binary at
 *          compile time. This means an edited migration that has already
 *          run will fail on checksum mismatch (sqlx compares the file hash
 *          against `_sqlx_migrations`), and a Docker image that reused a
 *          cached `COPY migrations` layer will run the OLD migrations.
 *          Rebuild with `--no-cache` after editing a migration file.
 *
 *          Migrations are applied inside a transaction per file. A failure
 *          rolls back that migration only; already-applied migrations are
 *          not re-run on the next startup. Fix the failing file and
 *          restart.
 *
 *      Deployment note:
 *          `run_migrations` is called synchronously during application
 *          boot. In a multi-replica deployment, this means N replicas may
 *          race to apply the same migration. The correct production
 *          pattern is a serialized release job that runs migrations
 *          before the application rollout (see DevPlan.md §10). For
 *          development and single-replica deployments, the current
 *          boot-time call is fine.
 *
 *      Known gaps (tracked in DataModel.md):
 *          - No retry loop around `connect` — a transient database
 *            startup delay fails the process. Acceptable given
 *            `depends_on: condition: service_healthy` in Compose.
 *          - No pool metrics exposed for observability.
 *          - Migrations run at boot in multi-replica deploys; needs
 *            externalized to a release job before production scale.
 * ============================================================================
 */
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

pub async fn init_pool(database_url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .connect(database_url)
        .await?;
    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

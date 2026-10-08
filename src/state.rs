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
 * File:        state.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-08
 * ============================================================================
 * Description:
 *      Shared application state passed to every Axum handler via
 *      `State<AppState>`. Bundles the four long-lived dependencies the
 *      service needs — database pool, Redis connection manager, object
 *      storage backend, and typed configuration — into a single Clone
 *      struct that Axum can move into each request.
 *
 *      Fields:
 *          - pool      sqlx PgPool. Async connection pool. Clone is cheap
 *                      (Arc internally); every handler gets its own handle
 *                      sharing the same underlying pool.
 *          - redis     redis::aio::ConnectionManager. Multiplexed Redis
 *                      client. Clone is cheap (Arc internally); safe to
 *                      share across concurrent requests.
 *          - storage   Arc<dyn ObjectStorage>. Trait object so tests can
 *                      substitute a fake and production can swap S3 /
 *                      rustfs / R2 without touching call sites.
 *          - config    Arc<AppConfig>. Immutable after boot. Arc so
 *                      handlers can read config fields without cloning
 *                      strings per request.
 *
 *      Construction:
 *          Built exactly once in `main.rs` after all four dependencies
 *          connect successfully. If any dependency fails to initialize,
 *          the process exits before `build_app` is called — there is no
 *          partial-boot state where the router serves requests with a
 *          missing dependency. See the "Startup sequence" in
 *          ARCHITECTURE.md.
 *
 *      Clone semantics:
 *          Every field is either a cheap handle or an Arc. `AppState` is
 *          `Clone` so Axum can move a copy into each request future
 *          without allocation beyond the Arc refcount bumps. Do not add
 *          fields that hold non-shared mutable state (e.g. a `Mutex<T>`
 *          used for cross-request coordination) without documenting the
 *          contention implications.
 *
 *      Testing:
 *          `TestApp` in `route_tests.rs` constructs `AppState` with a
 *          lazy PgPool (unreachable address), a mock Redis served by an
 *          in-process TCP listener, and a `TestStorage` implementation
 *          of `ObjectStorage`. Container-backed tests in
 *          `route_tests/database.rs` use a real Postgres via
 *          testcontainers, the same mock Redis, and the same fake
 *          storage. No code path in the test suite touches the dev
 *          database or the dev Redis.
 *
 *      Design notes:
 *          - No interior mutability. Every dependency is either
 *            synchronizing internally (PgPool, ConnectionManager) or
 *            immutable (config, storage trait object). Handlers do not
 *            need to coordinate access.
 *          - `storage` is behind a trait object, not a concrete type,
 *            so the object-storage backend is a runtime choice rather
 *            than a compile-time one. Enables the test fake without
 *            conditional compilation.
 *          - `config` is `Arc<AppConfig>`, not `AppConfig`. Cloning the
 *            config per request would copy every `String` field —
 *            unacceptable on the hot path.
 *          - No `Result` in the return type of `new`. By the time
 *            `AppState::new` is called, all four dependencies have
 *            already connected; the constructor is infallible.
 *
 *      Known gaps (tracked in DataModel.md and DEVPLAN.md):
 *          - No per-request customer context. Handlers that need
 *            `app.current_customer_id` for RLS (see migration 9) must
 *            set it via `SET LOCAL` on a per-request transaction. Not
 *            wired yet.
 *          - No request-scoped correlation ID. `correlation_id` in the
 *            event store and `request_log` is populated by tracing
 *            layers, not by `AppState`.
 *          - No feature-flag provider or app-version policy. Planned
 *            for the mobile delivery module (Epic E1, HU-027).
 *          - No access to a durable queue. Async work currently flows
 *            through the Postgres outbox (migration 3); a broker
 *            client would be a fifth field if introduced.
 *
 *      Not a serialized type. `AppState` never crosses an API boundary
 *      and has no `Serialize` / `Deserialize` impls.
 * ============================================================================
 */
use crate::config::AppConfig;
use crate::storage::ObjectStorage;
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub redis: ConnectionManager,
    pub storage: Arc<dyn ObjectStorage>,
    pub config: Arc<AppConfig>,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        redis: ConnectionManager,
        storage: Arc<dyn ObjectStorage>,
        config: Arc<AppConfig>,
    ) -> Self {
        Self {
            pool,
            redis,
            storage,
            config,
        }
    }
}

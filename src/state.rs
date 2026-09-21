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

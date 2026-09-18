use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "rho-studio-back-end",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn health_ready(State(state): State<AppState>) -> Json<Value> {
    let db_ok = sqlx::query("SELECT 1").fetch_one(&state.pool).await.is_ok();

    let mut redis_conn = state.redis.clone();
    let redis_ok = redis::cmd("PING")
        .query_async::<_, String>(&mut redis_conn)
        .await
        .map(|r| r == "PONG")
        .unwrap_or(false);

    let storage_ok = state.storage.public_url("__ping__").starts_with("http");

    let status = if db_ok && redis_ok && storage_ok { "ok" } else { "degraded" };

    Json(json!({
        "status": status,
        "checks": {
            "database": db_ok,
            "redis": redis_ok,
            "storage": storage_ok,
        }
    }))
}

pub async fn version() -> Json<Value> {
    Json(json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
    }))
}
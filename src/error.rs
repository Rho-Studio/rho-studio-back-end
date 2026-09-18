use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::NotFound(_)     => StatusCode::NOT_FOUND,
            AppError::BadRequest(_)   => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_)    => StatusCode::FORBIDDEN,
            AppError::Conflict(_)     => StatusCode::CONFLICT,
            AppError::Validation(_)   => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::Storage(_)
            | AppError::Database(_)
            | AppError::Redis(_)
            | AppError::Internal(_)   => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotFound(_)     => "NOT_FOUND",
            AppError::BadRequest(_)   => "BAD_REQUEST",
            AppError::Unauthorized(_) => "UNAUTHORIZED",
            AppError::Forbidden(_)    => "FORBIDDEN",
            AppError::Conflict(_)     => "CONFLICT",
            AppError::Validation(_)   => "VALIDATION_FAILED",
            AppError::Storage(_)      => "STORAGE_ERROR",
            AppError::Database(_)     => "DATABASE_ERROR",
            AppError::Redis(_)        => "CACHE_ERROR",
            AppError::Internal(_)     => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        }));

        if status.is_server_error() {
            tracing::error!(error = ?self, "server error");
        }

        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
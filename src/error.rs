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
 * File:        error.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-08
 * ============================================================================
 * Description:
 *      Application error type and its HTTP response mapping. Every handler
 *      in the crate returns `AppResult<T>` (an alias for
 *      `Result<T, AppError>`); the `IntoResponse` impl converts any
 *      `AppError` into a client-safe JSON body plus the right status code.
 *
 *      Two-message model:
 *          - Internal message  (`#[error("...")]` via thiserror)
 *                              Full detail. Used in `tracing::error!` for
 *                              server-side diagnosis. Never sent to the
 *                              client. May contain identifiers, provider
 *                              error strings, or SQL state.
 *          - Public message    (`public_message()`)
 *                              Static, safe string sent in the JSON body.
 *                              Same for every instance of a variant, so
 *                              it cannot leak instance-specific data.
 *      This split is the primary defense against information disclosure.
 *      See "Known gaps" below for the P0 items that close it out.
 *
 *      Variants:
 *          NotFound       404  Resource not found. Used for both genuinely
 *                              missing rows and cross-customer access, so
 *                              the API does not disclose existence.
 *          BadRequest     400  Malformed input the handler could not parse
 *                              (e.g., invalid JSON shape that Axum did not
 *                              already reject).
 *          Unauthorized   401  Missing, malformed, expired, or revoked
 *                              credentials. Deliberately vague on the wire.
 *          Forbidden      403  Authenticated but not permitted. Rare in
 *                              this codebase — most authorization failures
 *                              surface as 404 to avoid enumeration.
 *          Conflict       409  Request conflicts with current state (e.g.,
 *                              idempotency key reuse with a different body).
 *          Validation     422  Well-formed request that fails business or
 *                              shape validation (empty email, short
 *                              password, invalid CLABE).
 *          RateLimited    429  Carries `retry_after_seconds`; the
 *                              `IntoResponse` impl emits a `Retry-After`
 *                              header so mobile clients can back off.
 *          Unavailable    503  A required dependency (Redis, storage) is
 *                              reachable but not usable. Fail-closed:
 *                              rate-limit checks cannot proceed, so the
 *                              request is denied rather than allowed.
 *          Storage        500  `#[from] StorageError`. Object storage
 *                              failure (S3/rustfs).
 *          Database       500  `#[from] sqlx::Error`. Query or connection
 *                              failure. Mapped from `sqlx::Error` via `?`.
 *          Redis          500  `#[from] redis::RedisError`. Raw Redis
 *                              failure. Prefer `Unavailable` when the
 *                              intent is "dependency down, retry later".
 *          Internal       500  `#[from] anyhow::Error`. Catch-all for
 *                              programmer errors and unmapped third-party
 *                              failures. Never exposes the underlying
 *                              message on the wire.
 *
 *      HTTP mapping:
 *          `status()` maps each variant to a `StatusCode`. Client errors
 *          (4xx) return their variant-specific code; every server-side
 *          failure collapses to 500 with a generic body.
 *
 *      Stable error codes:
 *          `code()` returns a machine-readable string sent in the JSON
 *          body's `error.code` field. Mobile clients branch on these, so
 *          they are part of the public API contract — treat renames as
 *          breaking changes. Current values: `NOT_FOUND`, `BAD_REQUEST`,
 *          `UNAUTHORIZED`, `FORBIDDEN`, `CONFLICT`, `VALIDATION_FAILED`,
 *          `RATE_LIMITED`, `SERVICE_UNAVAILABLE`, `STORAGE_ERROR`,
 *          `DATABASE_ERROR`, `CACHE_ERROR`, `INTERNAL_ERROR`.
 *
 *      Response shape:
 *          `{"error": {"code": "...", "message": "..."}}`
 *          The `message` is always `public_message()`, never the internal
 *          string. There is no field that reflects the internal error.
 *
 *      Logging:
 *          5xx responses log the full `Debug` of `self` at ERROR level.
 *          4xx responses are not logged here — the tracing middleware
 *          records the request outcome, and logging every validation
 *          failure would flood the log.
 *
 *      Retry-After:
 *          Only `RateLimited` sets the `Retry-After` header. The value is
 *          `retry_after_seconds` stringified as ASCII decimal, matching
 *          RFC 7231 §7.1.3 for the delta-seconds form.
 *
 *      Third-party error conversions:
 *          `anyhow::Error` is the escape hatch for error types that do
 *          not implement `std::error::Error` (notably
 *          `argon2::password_hash::Error`). Convert with
 *          `anyhow::anyhow!(err.to_string())` at the call site, then `?`
 *          into `AppError::Internal`.
 *
 *      Design notes:
 *          - Every variant carries a `String` (or a source error), never
 *            a structured code enum. This keeps the call site cheap but
 *            means error codes are declared in `code()` rather than at
 *            the variant. If codes need to be programmatic, promote them
 *            to the variant.
 *          - `public_message()` returns `&'static str`. This is a
 *            deliberate constraint: it forces the message to be a compile-
 *            time constant, so no instance data can be interpolated.
 *            Adding an `#[error("... {0} ...")]`-style public message is
 *            not possible without changing the signature.
 *          - `IntoResponse` is implemented here rather than in a
 *            middleware. This means every handler that returns
 *            `AppResult<T>` gets the correct mapping for free and the
 *            conversion is impossible to forget.
 *          - `AppResult<T>` alias exists so handler signatures stay short
 *            and callers cannot accidentally return a raw `anyhow::Error`.
 *
 *      Known gaps (tracked in DataModel.md and DEVPLAN.md):
 *          - No structured error-code registry. Codes are declared in
 *            `code()` as string literals; a rename is a silent breaking
 *            change unless a test pins the values.
 *          - No correlation ID surfaced to the client. The body has no
 *            `correlation_id` field, so a support ticket cannot be tied
 *            to a specific request trace. Planned for HU-026.
 *          - No PII scrubbing on the internal message. `tracing::error!`
 *            logs the `Debug` of `self`, which for `Validation("...")`
 *            may include the offending value if a caller interpolated
 *            user input into the variant's `String`. The convention is
 *            to keep such strings generic; a CI lint is planned (P0 in
 *            DataModel.md).
 *          - No `Retry-After` on `Unavailable`. A 503 from a Redis
 *            outage could reasonably include a retry hint; currently only
 *            `RateLimited` does.
 *          - `RateLimited` does not classify by action. A login lockout
 *            and a signup throttle return the same code; a client cannot
 *            distinguish "wait 60 seconds" from "wait until tomorrow".
 *
 *      Not a serialized type. `AppError` never crosses the API boundary;
 *      only the JSON body produced by `into_response` does.
 * ============================================================================
 */
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
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

    #[error("rate limit exceeded")]
    RateLimited { retry_after_seconds: u64 },

    #[error("required dependency unavailable: {0}")]
    Unavailable(String),

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
    fn public_message(&self) -> &'static str {
        match self {
            AppError::NotFound(_) => "Resource not found",
            AppError::BadRequest(_) => "Invalid request",
            AppError::Unauthorized(_) => "Authentication failed",
            AppError::Forbidden(_) => "Not permitted",
            AppError::Conflict(_) => "Request conflicts with current state",
            AppError::Validation(_) => "Request validation failed",
            AppError::RateLimited { .. } => "Too many attempts; try again later",
            AppError::Unavailable(_) => "Service temporarily unavailable",
            AppError::Storage(_) => "Storage operation failed",
            AppError::Database(_) => "Database operation failed",
            AppError::Redis(_) => "Dependency operation failed",
            AppError::Internal(_) => "Internal server error",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            AppError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Storage(_)
            | AppError::Database(_)
            | AppError::Redis(_)
            | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::BadRequest(_) => "BAD_REQUEST",
            AppError::Unauthorized(_) => "UNAUTHORIZED",
            AppError::Forbidden(_) => "FORBIDDEN",
            AppError::Conflict(_) => "CONFLICT",
            AppError::Validation(_) => "VALIDATION_FAILED",
            AppError::RateLimited { .. } => "RATE_LIMITED",
            AppError::Unavailable(_) => "SERVICE_UNAVAILABLE",
            AppError::Storage(_) => "STORAGE_ERROR",
            AppError::Database(_) => "DATABASE_ERROR",
            AppError::Redis(_) => "CACHE_ERROR",
            AppError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let retry_after_seconds = match &self {
            AppError::RateLimited {
                retry_after_seconds,
            } => Some(*retry_after_seconds),
            _ => None,
        };
        let body = Json(json!({
            "error": {
                "code": self.code(),
                "message": self.public_message(),
            }
        }));

        if status.is_server_error() {
            tracing::error!(error = ?self, "server error");
        }

        let mut response = (status, body).into_response();
        if let Some(retry_after_seconds) = retry_after_seconds
            && let Ok(value) = retry_after_seconds.to_string().parse()
        {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        response
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::AppError;
    use axum::{http::header::RETRY_AFTER, response::IntoResponse};

    #[test]
    fn rate_limit_response_includes_retry_after() {
        let response = AppError::RateLimited {
            retry_after_seconds: 42,
        }
        .into_response();

        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[RETRY_AFTER], "42");
    }
}

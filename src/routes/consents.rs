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
 * File:        src/route/consents.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-11-04
 * ============================================================================
 * Description:
 *      LFPDPPP consent registry handlers. The platform can record verifiable consent
 *      evidence from day one, before any lending or financial workflow
 *      touches customer data.
 *
 *      Consent transition policy is separated into `crate::domain::consent`
 *      so that the state machine is testable without a database and is
 *      not entangled with HTTP concerns. This module is the transport
 *      layer: it authenticates, validates shape, captures audit evidence,
 *      opens a transaction, asks the domain whether to record an event, 
 *      and returns.
 *
 *      Endpoints:
 *          GET  /api/v1/consents/notices        Published notices for a locale
 *          GET  /api/v1/me/consents             Current consent state per user
 *          POST /api/v1/consents                Record a grant or withdrawal
 *          POST /api/v1/admin/consents/notices  Publish a new notice (RBAC)
 *
 *      Consent model (from migration 11):
 *          Two tables:
 *
 *              consent_notices          Versioned, locale-scoped notice
 *                                       content. Immutable once published.
 *                                       A partial unique index enforces at
 *                                       most one published notice per
 *                                       (purpose_code, locale).
 *              customer_consent_events  Append-only grant/withdraw evidence
 *                                       Includes audit fields: ip_address,
 *                                       user_agent, and consent_type (express/tacit).
 *                                       UPDATEand DELETE are blocked by trigger.
 *
 *          Current consent state for a (purpose, locale) pair is the
 *          latest event by (recorded_at DESC, id DESC). The
 *          `recorded_at` column defaults to `clock_timestamp()`, not
 *          `now()`, so events inserted in the same transaction get
 *          distinct timestamps and ordering is deterministic.
 *
 *      State machine & LFPDPPP Rules:
 *          The transition rules live in `domain::consent`. The
 *          handler determines if a purpose is "high risk" (e.g., KYC,
 *          Financial) and passes the user's requested `consent_type`
 *          to the domain layer:
 *
 *              - Granted on a published notice with no prior event    -> record
 *              - Granted for Financial/Sensitive data requires 
 *                Express consent (LFPDPPP Art. 7 & 8)                 -> reject if Tacit
 *              - Granted when the current state is already granted    -> no-op
 *              - Withdrawn when the current state is granted          -> record
 *              - Withdrawn when the current state is withdrawn        -> no-op
 *              - Withdrawn with no prior grant                        -> BadRequest
 *              - Any action on a non-published notice                 -> Conflict
 *
 *          The decision is idempotent: duplicate submissions of the
 *          same action produce the same result and do not append a
 *          second event.
 *
 *      Notice hash verification:
 *          `consent_notices.content_sha256` is verified on read. The
 *          handler recomputes SHA-256(content) for every published
 *          notice returned and returns `AppError::Internal` if any
 *          digest does not match. This is a defense-in-depth check;
 *          the write-side trigger in migration 11 already rejects an
 *          insert where the hash and content disagree. A read-side
 *          mismatch indicates either a tampered row (which the trigger
 *          should have prevented) or a bug in the writer.
 *
 *      Content immutability:
 *          Published and retired notices cannot be updated or deleted.
 *          Two triggers in migration 11 enforce this:
 *
 *              protect_consent_notice()       rejects content or identity
 *                                             changes on published and
 *                                             retired rows; sets published_at
 *                                             and retired_at on transitions
 *              prevent_consent_notice_delete() rejects DELETE outright
 *
 *          Retire, don't delete. A replacement notice is a new row with
 *          a higher version; the previous row stays for audit.
 *
 *      Error mapping:
 *          `map_consent_error` translates the domain error type
 *          (`ConsentError`) into the HTTP-facing `AppError`. The domain
 *          module does not depend on the transport layer; this is where
 *          the two meet.
 *
 *              NoticeNotPublished    -> Conflict (409)
 *              NoActiveConsent       -> BadRequest (400)
 *              ExpressConsentRequired-> BadRequest (400) (LFPDPPP violation)
 *              UnknownEventType      -> Internal (500)
 *
 *      Design notes:
 *          - Every handler requires an authenticated access token.
 *          - Admin RBAC: `publish_notice` calls `require_admin()`, which 
 *            verifies the authenticated user_id maps to a row where 
 *            `users.is_admin = true`. 
 *          - Server-side hashing: When publishing, the backend calculates 
 *            the SHA-256 hash in Rust. It does NOT trust the client to 
 *            provide the hash.
 *          - Strict locale validation: The `locale` parameter is validated 
 *            against a strict regex (`^[a-z]{2}-[A-Z]{2}$`) to prevent 
 *            injection attacks via the query string.
 *          - Audit Evidence: `record_consent` extracts the client IP
 *            (via `ConnectInfo` and trusted proxy logic) and `User-Agent`.
 *            This non-repudiation evidence is stored in the event log.
 *          - `record_consent` runs inside a transaction with a
 *            `FOR UPDATE` lock on the user row. This serializes
 *            concurrent consent submissions from the same user so the
 *            "current state" read and the "record event" write cannot
 *            race. Two simultaneous grants resolve to one event.
 *          - `GET /me/consents` uses `DISTINCT ON (purpose_code, locale)`
 *            with a DESC order on `(recorded_at, id)`.
 *          - `record_consent` returns `204 No Content` on success.
 *          - The `map_consent_error` mapping for `NoActiveConsent`
 *            returns `BadRequest` (400). This is a naming and status
 *            choice, not a bug.
 *
 *      Testing:
 *          Container-backed tests in `route_tests/database.rs` exercise
 *          the full consent flow, admin publishing RBAC, hash tampering,
 *          and LFPDPPP Express Consent enforcement. Unit tests for the 
 *          transition policy live in `crate::domain::consent`.
 *
 *      Known gaps (deferred to Phase 3, Feature F3 in DevPlan.md):
 *          - No re-consent prompt when a notice version is retired
 *            and replaced. Users with prior consent are not re-prompted.
 *          - No notification on new notice version.
 *          - No deduplication at the DB level. The domain check makes
 *            duplicate events a no-op in the handler, but nothing in
 *            the schema prevents a direct INSERT of two identical
 *            consecutive events.
 *
 *      Explicitly out of Phase 1 scope:
 *          - LFPDPPP ARCO request handling. Scheduled for F3.
 *          - Consent-driven data export and anonymization flows. Scheduled for F3.
 * ============================================================================
 */
use crate::domain::consent::{ConsentAction, ConsentError, 
                            should_record_event, ConsentType};
use crate::error::{AppError, AppResult};
use crate::routes::auth;
use crate::state::AppState;
use axum::{
    Json,
    extract::{ConnectInfo,Query, State},
    http::{HeaderMap, StatusCode, header::USER_AGENT}
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::net::SocketAddr;
use uuid::Uuid;
use once_cell::sync::Lazy; use regex::Regex;

#[derive(Debug, Deserialize)]
pub struct PublishNoticeRequest {
    pub purpose_code: String,
    pub locale: String,
    pub version: String,
    pub content: String,
}

#[derive(Debug, sqlx::FromRow, serde::Serialize)]
pub struct PublishedNoticeResponse {
    pub id: Uuid,
    pub purpose_code: String,
    pub locale: String,
    pub version: String,
    pub content_sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct NoticeQuery {
    locale: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConsentRequest {
    purpose_code: String,
    locale: String,
    version: String,
    action: ConsentAction,
    consent_type: ConsentType, //LFPDPPP Art.7
}

#[derive(Debug, FromRow, Serialize)]
pub struct ConsentNoticeResponse {
    id: Uuid,
    purpose_code: String,
    locale: String,
    version: String,
    content: String,
    content_sha256: String,
}

#[derive(Debug, FromRow, Serialize)]
pub struct ConsentStateResponse {
    purpose_code: String,
    locale: String,
    version: String,
    action: String,
    recorded_at: DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct NoticeState {
    id: Uuid,
    status: String,
}

fn map_consent_error(error: ConsentError) -> AppError {
    match error {
        ConsentError::NoticeNotPublished => {
            AppError::Conflict("Consent notice is not currently published".to_string())
        }
        ConsentError::NoActiveConsent => {
            AppError::BadRequest("No active consent exists for this notice".to_string())
        }
        ConsentError::UnknownEventType(event_type) => {
            AppError::Internal(anyhow::anyhow!("unknown consent event type: {event_type}"))
        }
        ConsentError::ExpressConsentRequired => {
            AppError::BadRequest("Sensitive data requires express consent (LFPDPPP Art.7)".to_string())
        }
    }
}

async fn require_admin(state: &AppState, user_id: Uuid) -> AppResult<()> {
    // Query the database for the user's admin status.
    // check `is_active = true` to prevent banned admins from publishing.
    let is_admin: Option<bool> = sqlx::query_scalar(
        "SELECT is_admin FROM users WHERE id = $1 AND is_active = true"
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;

    match is_admin {
        Some(true) => Ok(()),
        Some(false) => Err(AppError::Forbidden("Admin privileges required".to_string())),
        None => Err(AppError::Unauthorized("User not found or inactive".to_string())),
    }
}

pub async fn publish_notice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PublishNoticeRequest>,
) -> AppResult<(StatusCode, Json<PublishedNoticeResponse>)> {
    // 1. Authenticate: Get the user_id from the JWT (returns 401 if invalid)
    let user_id = crate::routes::auth::authenticated_user_id(&state, crate::routes::auth::bearer_token(&headers)?).await?;

    // 2. Authorize: Verify the user is an admin in the database (returns 403 if not)
    require_admin(&state, user_id).await?;

    // 3. Validate input shape
    if request.purpose_code.is_empty() || request.purpose_code.len() > 64
        || request.locale.is_empty() || request.locale.len() > 16
        || request.version.is_empty() || request.version.len() > 64
        || request.content.is_empty() 
    {
        return Err(AppError::Validation("Invalid notice payload".to_string()));
    }

    // 4. Hash the content (The backend calculates this, never trusting the client)
    let content_hash = format!("{:x}", Sha256::digest(request.content.as_bytes()));

    // 5. Insert into the database
    let insert_result = sqlx::query_as::<_, PublishedNoticeResponse>(
        r#"
        INSERT INTO consent_notices (
            purpose_code, locale, version, content, content_sha256, status, published_at
        )
        VALUES ($1, $2, $3, $4, $5, 'published', now())
        RETURNING id, purpose_code, locale, version, content_sha256
        "#,
    )
    .bind(&request.purpose_code)
    .bind(&request.locale)
    .bind(&request.version)
    .bind(&request.content)
    .bind(&content_hash)
    .fetch_one(&state.pool)
    .await;

    // 6. Handle database constraints
    match insert_result {
        Ok(notice) => {
            // Log the admin action for auditing
            tracing::info!(
                admin_id = %user_id, 
                notice_id = %notice.id, 
                purpose = %notice.purpose_code, 
                "Admin published a new privacy notice"
            );
            Ok((StatusCode::CREATED, Json(notice)))
        }
        Err(sqlx::Error::Database(db_error)) if db_error.is_unique_violation() => {
            // The partial unique index prevents two published notices for the same purpose/locale
            Err(AppError::Conflict(
                "A published notice already exists for this purpose and locale. Retire the existing notice first.".to_string()
            ))
        }
        Err(error) => {
            tracing::error!(error = ?error, "Failed to insert consent notice");
            Err(AppError::Internal(error.into()))
        }
    }
}

pub async fn get_published_notices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NoticeQuery>,
) -> AppResult<Json<Vec<ConsentNoticeResponse>>> {
    static LOCALE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z]{2}-[A-Z]{2}$").unwrap());
    let _user_id = auth::authenticated_user_id(&state, auth::bearer_token(&headers)?).await?;
    let locale = query.locale.unwrap_or_else(|| "es-MX".to_string());
    if !LOCALE_REGEX.is_match(&locale) {
        return Err(AppError::Validation("Invalid locale format".to_string()));
    }

    let notices = sqlx::query_as::<_, ConsentNoticeResponse>(
        r#"
        SELECT id, purpose_code, locale, version, content, content_sha256
        FROM consent_notices
        WHERE status = 'published' AND locale = $1
        ORDER BY purpose_code
        "#,
    )
    .bind(locale)
    .fetch_all(&state.pool)
    .await?;

    for notice in &notices {
        let actual_hash = format!("{:x}", Sha256::digest(notice.content.as_bytes()));
        if actual_hash != notice.content_sha256 {
            tracing::error!(
                notice_id = %notice.id,
                expected_hash = %notice.content_sha256,
                actual_hash = %actual_hash,
                "CRITICAL: published consent notice content hash mismatch — possible database tampering"
            );
            return Err(AppError::Internal(anyhow::anyhow!(
                "Content integrity verification failed"
            )));
        }
    }

    Ok(Json(notices))
}

pub async fn get_my_consents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<ConsentStateResponse>>> {
    let user_id = auth::authenticated_user_id(&state, auth::bearer_token(&headers)?).await?;
    let consents = sqlx::query_as::<_, ConsentStateResponse>(
        r#"
        SELECT DISTINCT ON (n.purpose_code, n.locale)
            n.purpose_code,
            n.locale,
            n.version,
            e.event_type AS action,
            e.recorded_at
        FROM customer_consent_events e
        JOIN consent_notices n ON n.id = e.notice_id
        WHERE e.user_id = $1
        ORDER BY n.purpose_code, n.locale, e.recorded_at DESC, e.id DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(consents))
}

pub async fn record_consent(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ConsentRequest>,
) -> AppResult<StatusCode> {
    let user_id = auth::authenticated_user_id(&state, auth::bearer_token(&headers)?).await?;
    if request.purpose_code.is_empty()
        || request.purpose_code.len() > 64
        || request.locale.is_empty()
        || request.locale.len() > 16
        || request.version.is_empty()
        || request.version.len() > 64
    {
        return Err(AppError::Validation(
            "Invalid consent reference".to_string(),
        ));
    }

    //Extract Audit Evidence
    let ip_address = auth::client_ip(
        peer,
        &headers,
        &state.config.auth_rate_limits.trusted_proxy_cidrs,
    );
    
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok());

    let mut transaction = state.pool.begin().await?;
    let active_user = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE id = $1 AND is_active = true AND email_verified = true FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await?;

    if active_user.is_none() {
        return Err(AppError::Unauthorized("Authentication failed".to_string()));
    }

    let notice = sqlx::query_as::<_, NoticeState>(
        r#"
        SELECT id, status
        FROM consent_notices
        WHERE purpose_code = $1 AND locale = $2 AND version = $3
        "#,
    )
    .bind(&request.purpose_code)
    .bind(&request.locale)
    .bind(&request.version)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::NotFound("Consent notice not found".to_string()))?;

    let current_action = sqlx::query_scalar::<_, String>(
        r#"
        SELECT event_type
        FROM customer_consent_events
        WHERE user_id = $1 AND notice_id = $2
        ORDER BY recorded_at DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .bind(notice.id)
    .fetch_optional(&mut *transaction)
    .await?
    .as_deref()
    .map(ConsentAction::from_event_type)
    .map(|result| result.map_err(map_consent_error))
    .transpose()?;

    //Determine Risk Level (KYC and Financial data are high risk)
    let is_high_risk = matches!(
        request.purpose_code.as_str(), 
        "kyc_verification" | "financial_data"
    );

    let should_record = should_record_event(
        current_action,
        request.action,
        notice.status == "published",
        is_high_risk,
        request.consent_type,
    ).map_err(map_consent_error)?;

    if should_record {
        sqlx::query(
            r#"
            INSERT INTO customer_consent_events 
            (user_id, notice_id, event_type, ip_address, user_agent, consent_type) 
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(user_id)
        .bind(notice.id)
        .bind(request.action.as_event_type())
        .bind(ip_address.to_string())
        .bind(user_agent)
        .bind(request.consent_type.as_str())
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

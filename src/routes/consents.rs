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
 * Date:        2026-10-12
 * ============================================================================
 * Description:
 *      LFPDPPP consent registry handlers. This is the second of the two
 *      features delivered in Phase 1: it ships alongside the identity
 *      module so that the platform can record verifiable consent
 *      evidence from day one, before any lending or financial workflow
 *      touches customer data.
 *
 *      Consent transition policy is separated into `crate::domain::consent`
 *      so that the state machine is testable without a database and is
 *      not entangled with HTTP concerns. This module is the transport
 *      layer: it authenticates, validates shape, opens a transaction,
 *      asks the domain whether to record an event, and returns.
 *
 *      Endpoints:
 *          GET  /api/v1/consents/notices  Published notices for a locale
 *          GET  /api/v1/me/consents       Current consent state per user
 *          POST /api/v1/consents          Record a grant or withdrawal
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
 *                                       per (user_id, notice_id). UPDATE
 *                                       and DELETE are blocked by trigger.
 *
 *          Current consent state for a (purpose, locale) pair is the
 *          latest event by (recorded_at DESC, id DESC). The
 *          `recorded_at` column defaults to `clock_timestamp()`, not
 *          `now()`, so events inserted in the same transaction get
 *          distinct timestamps and ordering is deterministic.
 *
 *      State machine:
 *          The transition rules live in `crate::domain::consent`. The
 *          handler asks the domain whether an event should be recorded
 *          given the current state, the requested action, and whether
 *          the notice is still published:
 *
 *              - Granted on a published notice with no prior event    -> record
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
 *              UnknownEventType      -> Internal (500) — indicates a row
 *                                       with an event_type value the domain
 *                                       does not recognize, which can only
 *                                       happen if the CHECK constraint in
 *                                       migration 11 has been bypassed or
 *                                       the domain enum has drifted from
 *                                       the schema.
 *
 *      Design notes:
 *          - Every handler requires an authenticated access token. The
 *            registry is user-scoped even for the notice list, so a
 *            caller must be a verified user. A future public-notices
 *            endpoint (pre-login) would need a separate route and a
 *            different threat model.
 *          - `record_consent` runs inside a transaction with a
 *            `FOR UPDATE` lock on the user row. This serializes
 *            concurrent consent submissions from the same user so the
 *            "current state" read and the "record event" write cannot
 *            race. Two simultaneous grants resolve to one event.
 *          - Locale, purpose_code, and version are validated for shape
 *            (non-empty, within column width) before the notice lookup.
 *            A shape failure returns `Validation`; a well-formed but
 *            nonexistent reference returns `NotFound`. This
 *            distinguishes client bugs from client mistakes.
 *          - `GET /me/consents` uses `DISTINCT ON (purpose_code, locale)`
 *            with a DESC order on `(recorded_at, id)`. That gives the
 *            current state per (purpose, locale) in a single query with
 *            an index-only scan against the
 *            `customer_consent_events_latest_idx` index.
 *          - The response shape of `GET /consents/notices` includes the
 *            full notice content. This is intentional — the mobile
 *            client needs to display the notice to obtain consent. The
 *            content is public by design once published.
 *          - `record_consent` returns `204 No Content` on success,
 *            whether the request caused an event insertion or was a
 *            no-op. The client does not need to distinguish; the
 *            subsequent `GET /me/consents` reflects the state.
 *          - The content hash check on read is O(n) in the number of
 *            published notices returned. For the initial locale set
 *            (single-digit notices) this is trivial; for a large
 *            regulatory catalogue it should move to a periodic
 *            verification job instead of a per-request check.
 *          - The `map_consent_error` mapping for `NoActiveConsent`
 *            returns `BadRequest` (400). This is a naming and status
 *            choice, not a bug: the request is well-formed but violates
 *            the current resource state. A future revision may switch
 *            to `Conflict` (409) for consistency with the idempotency
 *            contract on other endpoints.
 *
 *      Testing:
 *          Container-backed tests in `route_tests/database.rs` exercise
 *          the full consent flow: seed a published notice, list it,
 *          read the empty initial state, grant, verify idempotent
 *          re-grant, withdraw, verify idempotent re-withdraw, and
 *          assert event counts at each step. Unit tests for the
 *          transition policy live in `crate::domain::consent`.
 *          Integration tests for unauthenticated access live in
 *          `route_tests.rs`.
 *
 *      Known gaps (deferred to Phase 3, Feature F3 in DevPlan.md):
 *          - No seeder for the initial privacy and KYC notices. A
 *            fresh environment has an empty `consent_notices` table
 *            and the consent flow returns nothing. HU-012 scope.
 *          - No re-consent prompt when a notice version is retired
 *            and replaced. Users with prior consent are not re-prompted.
 *          - No notification on new notice version.
 *          - No deduplication at the DB level. The domain check makes
 *            duplicate events a no-op in the handler, but nothing in
 *            the schema prevents a direct INSERT of two identical
 *            consecutive events.
 *
 *      Explicitly out of Phase 1 scope:
 *          - LFPDPPP ARCO request handling (access, rectification,
 *            cancellation, opposition). Schema exists in migration 5;
 *            no handler. Scheduled for F3.
 *          - Consent-driven data export and anonymization flows.
 *            Scheduled for F3.
 *
 *      Not a serialized type. `ConsentNoticeResponse` and
 *      `ConsentStateResponse` implement `Serialize`; `NoticeState` is
 *      internal to the handler.
 * ============================================================================
 */
use crate::domain::consent::{ConsentAction, ConsentError, should_record_event};
use crate::error::{AppError, AppResult};
use crate::routes::auth;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

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
    }
}

pub async fn get_published_notices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NoticeQuery>,
) -> AppResult<Json<Vec<ConsentNoticeResponse>>> {
    let _user_id = auth::authenticated_user_id(&state, auth::bearer_token(&headers)?).await?;
    let locale = query.locale.unwrap_or_else(|| "es-MX".to_string());
    if locale.is_empty() || locale.len() > 16 {
        return Err(AppError::Validation("Invalid locale".to_string()));
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
            return Err(AppError::Internal(anyhow::anyhow!(
                "published consent notice content hash mismatch"
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

    let should_record =
        should_record_event(current_action, request.action, notice.status == "published")
            .map_err(map_consent_error)?;
    if should_record {
        sqlx::query(
            "INSERT INTO customer_consent_events (user_id, notice_id, event_type) VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(notice.id)
        .bind(request.action.as_event_type())
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

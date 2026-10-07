-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        8_audit_and_operations.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Operational and audit tables. Distinct from the event store: these
--      capture operational metadata (durable idempotency, HTTP request log,
--      projection checkpoints, regulatory calendar), not domain facts.
--
--      Tables:
--          - idempotency_keys        Durable fallback for idempotency keys.
--                                    Redis is primary; Postgres survives
--                                    Redis restarts.
--          - request_log             Structured HTTP request log. Separate
--                                    from domain_events; captures every
--                                    request, not just those producing
--                                    domain events.
--          - projection_checkpoints  Last processed event per projection.
--                                    Used to resume after restart.
--          - regulatory_calendar     Filing deadlines (CNBV, UIF, CONDUSEF).
--
--      Depends on:
--          - migration 1: pgcrypto (gen_random_uuid)
--
--      Design notes:
--          - Durable idempotency: Redis primary + Postgres fallback. The same
--            key space exists in both stores; the app checks both.
--          - Response caching for idempotent replay: response_status and
--            response_body cached so a duplicate returns the original result
--            without re-executing.
--          - request_hash guards against key reuse with a different body.
--          - Separation of concerns: request_log != domain_events. Operational
--            metadata vs. domain facts; different lifecycle and access.
--          - Checkpoint pattern: last_event_id + last_version enables
--            resume-from-event-log.
--          - Seed idempotency: ON CONFLICT DO NOTHING for both seeds.
--
--      Known gaps (tracked in DataModel.md):
--          - No middleware populates request_log or idempotency_keys.
--          - No cleanup job for expired idempotency keys.
--          - No projection runner updates projection_checkpoints.
--          - No filing scheduler updates regulatory_calendar.next_due_date.
--          - No archival policy for request_log.
--          - projection_checkpoints lacks a last_error column.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- Idempotency Keys — Redis is the primary store, but PostgreSQL provides
-- a durable fallback for keys that must survive Redis restarts.
-- ---------------------------------------------------------------------------
CREATE TABLE idempotency_keys (
    key                 UUID PRIMARY KEY,
    customer_id         UUID NOT NULL,
    endpoint            VARCHAR(255) NOT NULL,
    request_hash        VARCHAR(64) NOT NULL,     -- SHA-256 of the request body
    response_status     INTEGER,
    response_body       JSONB,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '24 hours'
);

CREATE INDEX idx_idempotency_expiry ON idempotency_keys (expires_at);

-- ---------------------------------------------------------------------------
-- Structured Request Log — for mobile debugging and monitoring.
-- This is separate from the event store. It captures every HTTP request
-- regardless of whether it produced a domain event.
-- ---------------------------------------------------------------------------
CREATE TABLE request_log (
    id                  BIGSERIAL PRIMARY KEY,
    correlation_id      UUID NOT NULL,
    customer_id         UUID,
    device_id           UUID,
    app_version         VARCHAR(20),
    platform            VARCHAR(10),
    os_version          VARCHAR(20),
    endpoint            VARCHAR(255) NOT NULL,
    method              VARCHAR(10) NOT NULL,
    status_code         INTEGER NOT NULL,
    duration_ms         INTEGER NOT NULL,
    response_size_bytes INTEGER,
    error_code          VARCHAR(50),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_request_log_correlation ON request_log (correlation_id);
CREATE INDEX idx_request_log_customer_time ON request_log (customer_id, created_at DESC);
CREATE INDEX idx_request_log_errors ON request_log (status_code, created_at DESC)
    WHERE status_code >= 400;

-- ---------------------------------------------------------------------------
-- Projection Checkpoints — tracks the last processed event per projection.
-- Used by the projection handler to resume after a restart.
-- ---------------------------------------------------------------------------
CREATE TABLE projection_checkpoints (
    projection_name     VARCHAR(100) PRIMARY KEY,
    last_event_id       UUID,
    last_occurred_at    TIMESTAMPTZ,
    last_version        BIGINT,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Seed with the projections that exist.
INSERT INTO projection_checkpoints (projection_name) VALUES
    ('account_balances'),
    ('loan_summary'),
    ('transaction_history'),
    ('sync_changes'),
    ('delinquency_report')
ON CONFLICT (projection_name) DO NOTHING;

-- ---------------------------------------------------------------------------
-- Regulatory Calendar — scheduled filing deadlines.
-- ---------------------------------------------------------------------------
CREATE TABLE regulatory_calendar (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    report_name         VARCHAR(100) NOT NULL,
    authority           VARCHAR(20) NOT NULL,     -- cnbv | uif | condusef
    frequency           VARCHAR(20) NOT NULL,     -- monthly | quarterly | annual
    due_day             INTEGER,                  -- day of the following month/quarter
    next_due_date       DATE NOT NULL,
    last_submitted_at   TIMESTAMPTZ,
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    notes               TEXT
);

-- Seed the known reporting deadlines.
INSERT INTO regulatory_calendar (report_name, authority, frequency, due_day, next_due_date) VALUES
    ('Capital adequacy', 'cnbv', 'monthly', 15, CURRENT_DATE + INTERVAL '30 days'),
    ('Financial statements', 'cnbv', 'quarterly', 30, CURRENT_DATE + INTERVAL '90 days'),
    ('PLD monthly report', 'uif', 'monthly', 17, CURRENT_DATE + INTERVAL '30 days'),
    ('Customer complaints', 'condusef', 'monthly', 10, CURRENT_DATE + INTERVAL '30 days');
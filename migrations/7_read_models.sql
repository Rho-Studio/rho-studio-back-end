-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        7_read_models.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Denormalized CQRS projections maintained by event handlers. Queries
--      hit these tables; they never touch the event store or the current-
--      state aggregate tables for read operations.
--
--      Tables:
--          - account_balances       Balance / available / held per account.
--          - loan_summary           Loan state summary with next-payment and
--                                   days-past-due for query without joins.
--          - transaction_history    Immutable ledger of account activity.
--                                   Safe to cache indefinitely on the client.
--          - sync_changes           Incremental sync cursor for mobile.
--          - delinquency_report     Delinquency metrics per loan.
--
--      Depends on:
--          - migration 1: pgcrypto, loan_status enum
--          - migration 3: logical dependency on domain_events (projections
--            consume events; no FK constraint)
--
--      Design notes:
--          - Denormalization is deliberate. available_mxn is balance - held
--            but stored; recomputing per query would defeat the projection.
--          - last_event_version + last_event_id on mutable projections so a
--            projector can detect gaps or replays.
--          - Append-only where semantics allow. transaction_history and
--            sync_changes never UPDATE.
--          - BIGSERIAL for cursor tables. sync_changes.id is a monotonic
--            sequence because the sync protocol needs ordering, not UUID
--            identity.
--          - Partial indexes for active subsets. delinquency_report filters
--            days_past_due > 0.
--          - Projection PK = source entity PK. Enables natural upserts via
--            ON CONFLICT (account_id) DO UPDATE.
--
--      Known gaps (tracked in DataModel.md):
--          - No projector. Tables are never populated.
--          - No /sync endpoint. sync_changes has no consumer.
--          - No backfill script for replaying historical events.
--          - last_event_version is not enforced as monotonic.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- Account Balances Projection
-- Updated on FundsCredited, FundsDebited, HoldPlaced, HoldReleased.
-- ---------------------------------------------------------------------------
CREATE TABLE account_balances (
    account_id          UUID PRIMARY KEY,
    customer_id         UUID NOT NULL,
    balance_mxn         NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    available_mxn       NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    held_mxn            NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    last_event_version  BIGINT NOT NULL DEFAULT 0,
    last_event_id       UUID,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_account_balances_customer ON account_balances (customer_id);

-- ---------------------------------------------------------------------------
-- Loan Summary Projection
-- Updated on LoanDisbursed, RepaymentRecorded, LoanPaidOff, LoanDelinquent.
-- ---------------------------------------------------------------------------
CREATE TABLE loan_summary (
    loan_id             UUID PRIMARY KEY,
    customer_id         UUID NOT NULL,
    loan_number         VARCHAR(30) NOT NULL,
    principal_mxn       NUMERIC(18,2) NOT NULL,
    outstanding_mxn     NUMERIC(18,2) NOT NULL,
    interest_rate_bps   INTEGER NOT NULL,
    term_months         INTEGER NOT NULL,
    status              loan_status NOT NULL,
    next_payment_date   DATE,
    next_payment_amount NUMERIC(18,2),
    payments_made       INTEGER NOT NULL DEFAULT 0,
    days_past_due       INTEGER NOT NULL DEFAULT 0,
    last_event_version  BIGINT NOT NULL DEFAULT 0,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_loan_summary_customer ON loan_summary (customer_id);
CREATE INDEX idx_loan_summary_status ON loan_summary (status);

-- ---------------------------------------------------------------------------
-- Transaction History Projection
-- Append-only. Immutable rows. Safe to cache indefinitely on the mobile
-- client. Every financial event that affects an account produces a row.
-- ---------------------------------------------------------------------------
CREATE TABLE transaction_history (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL,
    customer_id         UUID NOT NULL,
    event_id            UUID NOT NULL,
    event_type          VARCHAR(100) NOT NULL,
    amount_mxn          NUMERIC(18,2),
    running_balance_mxn NUMERIC(18,2),
    description         VARCHAR(255),
    counterparty        VARCHAR(200),
    reference           VARCHAR(100),
    occurred_at         TIMESTAMPTZ NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_txn_history_account ON transaction_history (account_id, occurred_at DESC);
CREATE INDEX idx_txn_history_customer ON transaction_history (customer_id, occurred_at DESC);

-- ---------------------------------------------------------------------------
-- Sync Changes — incremental sync watermark for mobile clients
-- Records which read-model rows changed in response to which event.
-- The /sync endpoint queries this table with a cursor on occurred_at.
-- ---------------------------------------------------------------------------
CREATE TABLE sync_changes (
    id                  BIGSERIAL PRIMARY KEY,
    entity_type         VARCHAR(50) NOT NULL,     -- account_balances | loan_summary | ...
    entity_id           UUID NOT NULL,
    customer_id         UUID NOT NULL,
    change_type         VARCHAR(10) NOT NULL,     -- upsert | delete
    occurred_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    event_id            UUID NOT NULL
);

CREATE INDEX idx_sync_changes_customer ON sync_changes (customer_id, occurred_at);
CREATE INDEX idx_sync_changes_cursor ON sync_changes (occurred_at, id);

-- ---------------------------------------------------------------------------
-- Delinquency Report Projection
-- Updated on InterestAccrued, LoanDelinquent.
-- ---------------------------------------------------------------------------
CREATE TABLE delinquency_report (
    loan_id             UUID PRIMARY KEY,
    customer_id         UUID NOT NULL,
    days_past_due       INTEGER NOT NULL DEFAULT 0,
    overdue_amount_mxn  NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    last_payment_date   DATE,
    last_event_version  BIGINT NOT NULL DEFAULT 0,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_delinquency_dpd ON delinquency_report (days_past_due DESC)
    WHERE days_past_due > 0;
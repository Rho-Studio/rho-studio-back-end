-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        6_payment_rails.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Banxico payment rail tables. SPEI, CoDi, and DiMo share a single
--      spei_transfers table discriminated by the rail enum; rail-specific
--      fields live in codi_messages and dimo_registrations. Transfer counters
--      per account/period live in transfer_limits.
--
--      Tables:
--          - spei_transfers        Shared transfer record for all three rails.
--                                  Idempotency key unique. CLABE format check
--                                  only (18 digits, no check digit validation).
--          - codi_messages         CoDi QR / message lifecycle. Optional FK
--                                  to spei_transfers.
--          - dimo_registrations    Phone-to-CLABE mapping. Unique per
--                                  (phone_number, institution_code).
--          - transfer_limits       Daily / monthly counters per account.
--
--      Depends on:
--          - migration 1: pgcrypto, enums payment_rail, transfer_status
--          - migration 2: accounts(id), customers(id) FKs
--
--      Design notes:
--          - Single-table inheritance. One spei_transfers table for all
--            rails; rail-specific fields in satellite tables. Avoids
--            duplicating shared columns.
--          - Idempotency as a first-class column. idempotency_key UUID
--            UNIQUE prevents mobile retries from double-charging.
--          - Regulatory fields captured at event time. udi_value_at_time
--            and amount_udis stored on the row, not derived later.
--          - Nullable FKs for optional links. codi_messages.spei_transfer_id
--            allows a message to exist before settlement.
--          - Partial indexes for active subsets. dimo_registrations filters
--            is_active; spei_transfers indexes only non-null tracking keys.
--          - Counters, not limits. transfer_limits accumulates; the limits
--            themselves are enforced in the application.
--          - Money columns use NUMERIC(18,2) for MXN; UDI uses NUMERIC(12,6).
--            Never FLOAT or REAL.
--
--      Known gaps (tracked in DataModel.md):
--          - No handler reads or writes any table in this migration.
--          - CLABE check digit is not validated; format only.
--          - transfer_limits does not store the limits themselves.
--          - No reconciliation worker or settlement proof table.
--          - destination_clabe is not verified against beneficiary ownership.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- SPEI Transfers
-- CLABE is 18 digits. The last digit is a check digit computed with
-- weights 3-7-1 per Banxico specification.
-- ---------------------------------------------------------------------------
CREATE TABLE spei_transfers (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    origin_account_id   UUID NOT NULL REFERENCES accounts(id),
    destination_clabe   VARCHAR(18) NOT NULL,
    destination_name    VARCHAR(200),
    amount_mxn          NUMERIC(18,2) NOT NULL,
    currency            CHAR(3) NOT NULL DEFAULT 'MXN',
    concept             VARCHAR(40) NOT NULL,
    reference           VARCHAR(40),
    rail                payment_rail NOT NULL DEFAULT 'spei',
    status              transfer_status NOT NULL DEFAULT 'initiated',
    -- Banxico tracking
    spei_tracking_key   VARCHAR(50),        -- Clave de rastreo SPEI
    banxico_reference   VARCHAR(50),
    -- Regulatory
    udi_value_at_time   NUMERIC(12,6),
    amount_udis         NUMERIC(18,6),
    -- Idempotency
    idempotency_key     UUID NOT NULL,
    -- Timestamps
    initiated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ,
    failed_at           TIMESTAMPTZ,
    failure_reason      TEXT,
    -- Audit
    correlation_id      UUID NOT NULL,
    device_id           UUID,

    CONSTRAINT spei_amount_positive CHECK (amount_mxn > 0),
    CONSTRAINT spei_concept_length CHECK (char_length(concept) <= 40),
    CONSTRAINT spei_clabe_format CHECK (destination_clabe ~ '^[0-9]{18}$'),
    CONSTRAINT spei_idempotency_unique UNIQUE (idempotency_key)
);

CREATE INDEX idx_spei_origin ON spei_transfers (origin_account_id, initiated_at DESC);
CREATE INDEX idx_spei_status ON spei_transfers (status);
CREATE INDEX idx_spei_tracking ON spei_transfers (spei_tracking_key)
    WHERE spei_tracking_key IS NOT NULL;

-- ---------------------------------------------------------------------------
-- CoDi Messages
-- Maximum message amount: 6,000 UDIs.
-- When originating from a push notification, beneficiary data cannot be edited.
-- ---------------------------------------------------------------------------
CREATE TABLE codi_messages (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    spei_transfer_id    UUID REFERENCES spei_transfers(id),
    message_type        VARCHAR(20) NOT NULL,     -- cobro | pago
    qr_payload          TEXT,
    amount_mxn          NUMERIC(18,2) NOT NULL,
    amount_udis         NUMERIC(18,6) NOT NULL,
    udi_value           NUMERIC(12,6) NOT NULL,
    -- Regulatory
    from_push_notification BOOLEAN NOT NULL DEFAULT FALSE,
    beneficiary_data_editable BOOLEAN NOT NULL DEFAULT TRUE,
    status              VARCHAR(20) NOT NULL DEFAULT 'pending',
    expires_at          TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT codi_max_udis CHECK (amount_udis <= 6000),
    CONSTRAINT codi_push_immutable CHECK (
        NOT (from_push_notification AND beneficiary_data_editable)
    )
);

-- ---------------------------------------------------------------------------
-- DiMo — Phone-to-CLABE Mapping
-- A single phone number can be linked to one account per bank, but the
-- same number can be registered with multiple institutions.
-- ---------------------------------------------------------------------------
CREATE TABLE dimo_registrations (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    phone_number        VARCHAR(10) NOT NULL,     -- 10-digit Mexican mobile
    clabe               VARCHAR(18) NOT NULL,
    institution_code    VARCHAR(5) NOT NULL,      -- Banxico institution code
    customer_id         UUID NOT NULL REFERENCES customers(id),
    account_id          UUID NOT NULL REFERENCES accounts(id),
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    registered_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    deactivated_at      TIMESTAMPTZ,

    CONSTRAINT dimo_unique UNIQUE (phone_number, institution_code),
    CONSTRAINT dimo_phone_format CHECK (phone_number ~ '^[0-9]{10}$')
);

CREATE INDEX idx_dimo_lookup ON dimo_registrations (phone_number, institution_code)
    WHERE is_active = TRUE;

-- ---------------------------------------------------------------------------
-- Transfer Limits — daily and monthly tracking per account
-- ---------------------------------------------------------------------------
CREATE TABLE transfer_limits (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id),
    period_type         VARCHAR(10) NOT NULL,     -- daily | monthly
    period_start        DATE NOT NULL,
    total_transferred   NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    total_transfers     INTEGER NOT NULL DEFAULT 0,
    last_transfer_at    TIMESTAMPTZ,

    CONSTRAINT transfer_limits_unique UNIQUE (account_id, period_type, period_start)
);

CREATE INDEX idx_transfer_limits_account ON transfer_limits (account_id, period_type, period_start);
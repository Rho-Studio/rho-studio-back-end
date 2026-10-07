-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        5_kyc_devices_and_arco.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      KYC evidence, LFPDPPP data-subject rights, and mobile device registry.
--      The legacy consent_records table and its consent_method enum were
--      removed during the migration-11 cleanup; consent evidence now lives
--      in customer_consent_events (user-scoped, migration 11). This file
--      retains only the tables that remain part of the target schema.
--
--      Tables:
--          - kyc_verifications       Append-only KYC evidence per
--                                    (customer, level). Document validations,
--                                    biometric checks, RENAPO/INE references.
--                                    Latest per (customer_id, level) is the
--                                    current state; prior rows are audit.
--          - arco_requests           LFPDPPP Access/Rectification/Cancellation
--                                    /Opposition requests. 20-business-day SLA
--                                    encoded as due_at default.
--          - data_anonymization_log  What was anonymized, when, and by whom
--                                    for each fulfilled cancellation ARCO.
--          - registered_devices      Hardware-backed public key + platform
--                                    attestation per mobile device.
--          - device_tokens           Push notification tokens per device.
--
--      Depends on:
--          - migration 1: pgcrypto, enums kyc_level, arco_request_type,
--            arco_status, device_status
--          - migration 2: customers(id) FK
--
--      Design notes:
--          - Append-only evidence. kyc_verifications and
--            data_anonymization_log are never updated; corrections are new
--            rows. Required for LFPDPPP and CNBV auditability.
--          - Soft delete via status / timestamp. registered_devices uses
--            status = 'revoked'; device_tokens uses invalidated_at. Rows
--            are retained so security teams can see history.
--          - Storage references, not blobs. proof_of_address_url and
--            response_document_url store S3 object keys (rustfs locally,
--            S3/R2 in production). Bytes never traverse Postgres.
--          - Regulatory cadences baked into DDL. arco_requests.due_at =
--            now() + INTERVAL '20 days' documents the LFPDPPP SLA in the
--            schema, not in code.
--          - Partial indexes for active subsets. arco_requests skips
--            completed/rejected/expired; device_tokens skips invalidated.
--          - Composite uniqueness captures real-world identity:
--            registered_devices (device_id, customer_id);
--            device_tokens (device_id, token).
--          - data_anonymization_log.arco_request_id is a FK; customer_id
--            is not — the whole point of anonymization is that the customer
--            may no longer exist.
--          - consent_records is commented out for historical reference.
--
--      Known gaps (tracked in DataModel.md):
--          - No handler reads or writes any table in this migration.
--          - customer_verification_cases (migration 12) is not linked to
--            kyc_verifications; the case and the evidence live in separate
--            unlinked models.
--          - registered_devices.risk_score has no update source after
--            initial insert.
--          - kyc_verifications has no updated_at; re-verification timestamps
--            only via created_at.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- KYC Verification Records
-- Each level has specific document requirements. Records are append-only
-- for audit purposes; the latest record per (customer, level) is the
-- current state.
-- ---------------------------------------------------------------------------
CREATE TABLE kyc_verifications (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL REFERENCES customers(id),
    level               kyc_level NOT NULL,
    status              VARCHAR(20) NOT NULL DEFAULT 'pending',  -- pending | approved | rejected
    -- Nivel 2: RFC, official ID
    rfc_validated       BOOLEAN NOT NULL DEFAULT FALSE,
    official_id_type    VARCHAR(30),
    official_id_number  VARCHAR(50),
    -- Nivel 3: CURP, proof of address
    curp_validated      BOOLEAN NOT NULL DEFAULT FALSE,
    proof_of_address_url TEXT,          -- MinIO object key
    -- Nivel 4: biometric
    biometric_verified  BOOLEAN NOT NULL DEFAULT FALSE,
    liveness_check_passed BOOLEAN,
    video_call_duration_seconds INTEGER,  -- min 30 seconds for Nivel 3/4 remote
    -- RENAPO / INE validation references
    renaapo_reference   VARCHAR(100),
    ine_validation_ref  VARCHAR(100),
    -- Audit
    verified_by         VARCHAR(100),
    verified_at         TIMESTAMPTZ,
    rejection_reason    TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT kyc_video_duration_min CHECK (
        video_call_duration_seconds IS NULL OR video_call_duration_seconds >= 30
    )
);

CREATE INDEX idx_kyc_customer_level ON kyc_verifications (customer_id, level, created_at DESC);

-- ---------------------------------------------------------------------------
-- LFPDPPP — Consent Records
-- Explicit consent is required for processing sensitive personal data.
-- Records are append-only. The domain records consent version, timestamp,
-- method, and IP address.
-- ---------------------------------------------------------------------------
-- CREATE TABLE consent_records (
--     id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
--     customer_id         UUID NOT NULL REFERENCES customers(id),
--     notice_version      VARCHAR(20) NOT NULL,
--     consented_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
--     method              consent_method NOT NULL,
--     ip_address          INET NOT NULL,
--     device_id           UUID,
--     purposes_accepted   JSONB NOT NULL DEFAULT '[]'::jsonb,  -- array of purpose strings
--     purposes_rejected   JSONB NOT NULL DEFAULT '[]'::jsonb,
--     withdrawn_at        TIMESTAMPTZ,
--     withdrawal_reason   TEXT
-- );

-- CREATE INDEX idx_consent_customer ON consent_records (customer_id, consented_at DESC);

-- ---------------------------------------------------------------------------
-- LFPDPPP — ARCO Requests
-- Access, Rectification, Cancellation, Opposition.
-- Must be serviced within 20 business days.
-- The 2025 reform allows legal entities to exercise these rights.
-- ---------------------------------------------------------------------------
CREATE TABLE arco_requests (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL REFERENCES customers(id),
    request_type        arco_request_type NOT NULL,
    status              arco_status NOT NULL DEFAULT 'received',
    -- For rectification
    field_to_rectify    VARCHAR(100),
    current_value       TEXT,
    requested_value     TEXT,
    -- For opposition
    purpose_to_oppose   VARCHAR(100),
    -- SLA tracking
    received_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    due_at              TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '20 days',
    completed_at        TIMESTAMPTZ,
    -- Response
    response_notes      TEXT,
    response_document_url TEXT,         -- MinIO object key for data export
    rejection_reason    TEXT,
    -- Audit
    handled_by          VARCHAR(100),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT arco_due_after_received CHECK (due_at > received_at)
);

CREATE INDEX idx_arco_customer ON arco_requests (customer_id, received_at DESC);
CREATE INDEX idx_arco_overdue ON arco_requests (due_at)
    WHERE status NOT IN ('completed', 'rejected', 'expired');

-- ---------------------------------------------------------------------------
-- Data Anonymization Log
-- When a cancellation ARCO request is fulfilled, personal data is
-- anonymized. This log records what was anonymized, when, and by whom.
-- The event store is never modified.
-- ---------------------------------------------------------------------------
CREATE TABLE data_anonymization_log (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL,
    arco_request_id     UUID NOT NULL REFERENCES arco_requests(id),
    fields_anonymized   JSONB NOT NULL,
    anonymized_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    anonymized_by       VARCHAR(100) NOT NULL
);

-- ---------------------------------------------------------------------------
-- Device Registration
-- Every mobile device is registered with a hardware-backed public key
-- and platform attestation.
-- ---------------------------------------------------------------------------
CREATE TABLE registered_devices (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    device_id           UUID NOT NULL,
    customer_id         UUID NOT NULL REFERENCES customers(id),
    platform            VARCHAR(10) NOT NULL,    -- ios | android
    os_version          VARCHAR(20),
    device_model        VARCHAR(50),
    public_key_pem      TEXT NOT NULL,
    attestation_status  VARCHAR(20) NOT NULL DEFAULT 'pending',
    attestation_token   TEXT,
    risk_score          NUMERIC(5,2) NOT NULL DEFAULT 0.00,
    status              device_status NOT NULL DEFAULT 'active',
    last_seen_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    registered_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at          TIMESTAMPTZ,

    CONSTRAINT device_unique UNIQUE (device_id, customer_id)
);

CREATE INDEX idx_devices_customer ON registered_devices (customer_id);
CREATE INDEX idx_devices_status ON registered_devices (status);

-- ---------------------------------------------------------------------------
-- Push Tokens
-- ---------------------------------------------------------------------------
CREATE TABLE device_tokens (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL REFERENCES customers(id),
    device_id           UUID NOT NULL,
    platform            VARCHAR(10) NOT NULL,
    token               TEXT NOT NULL,
    app_version         VARCHAR(20) NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    invalidated_at      TIMESTAMPTZ,

    CONSTRAINT device_token_unique UNIQUE (device_id, token)
);

CREATE INDEX idx_push_tokens_customer ON device_tokens (customer_id)
    WHERE invalidated_at IS NULL;
-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        1_extensions_and_types.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Foundation migration. Every other migration (2–13) assumes the
--      extensions and enums declared here are already present.
--
--      Extensions:
--          - pgcrypto    → gen_random_uuid() defaults on every PK;
--                          digest('sha256') used by the consent-notice trigger.
--          - pg_trgm     → trigram indexes for fuzzy customer-name search
--                          and future dedup when linking customers.user_id.
--          - btree_gin   → composite GIN indexes mixing scalars and JSONB
--                          (e.g. aml_risk_profiles.risk_factors).
--
--      Enums (declaration order is the ORDER BY sort order):
--          - kyc_level           Ley Fintech Art. 115 tiers.
--          - account_status      Account lifecycle, ordered by severity.
--          - loan_status         Loan lifecycle.
--          - application_status  Pre-loan application lifecycle.
--          - aml_risk_level      LFPIORPI risk classification.
--          - payment_rail        Banxico rails (SPEI, CoDi, DiMo).
--          - transfer_status     Transfer lifecycle.
--          - arco_request_type   LFPDPPP data-subject rights.
--          - arco_status         ARCO SLA lifecycle.
--          - device_status       Mobile device lifecycle.
--
--      Design notes:
--          - Native enums are used only where the vocabulary is
--            regulator-fixed and declaration-order sorting matters.
--            Everything else uses VARCHAR + CHECK for cheaper evolution.
--          - consent_method was removed during the migration 5 cleanup;
--            its commented block is kept for history.
--          - ALTER TYPE ... ADD VALUE cannot run inside a transaction on
--            older PostgreSQL — future enum changes must use a DO block
--            or recreate the type.
-- 
-- gen_random_uuid() requires pgcrypto on PostgreSQL < 13.
-- PostgreSQL 16 has it built-in, but pgcrypto is kept for gen_random_bytes().
-- ============================================================================

CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_trgm;       -- trigram search for customer lookup
CREATE EXTENSION IF NOT EXISTS btree_gin;     -- composite GIN indexes

-- ---------------------------------------------------------------------------
-- Enums for domain states
-- Using CHECK constraints instead of native ENUMs. CHECK constraints avoid
-- the migration cost of ALTER TYPE and are easier to evolve. Native ENUMs
-- are only preferable when declared sort order matters.
-- ---------------------------------------------------------------------------

-- Customer KYC levels per Ley Fintech Article 115
CREATE TYPE kyc_level AS ENUM ('nivel_1', 'nivel_2', 'nivel_3', 'nivel_4');

-- Account lifecycle
CREATE TYPE account_status AS ENUM (
    'active', 'suspended', 'closed', 'frozen'
);

-- Loan lifecycle
CREATE TYPE loan_status AS ENUM (
    'pending_disbursement', 'active', 'delinquent', 'paid_off', 'written_off'
);

-- Loan application lifecycle
CREATE TYPE application_status AS ENUM (
    'submitted', 'under_review', 'approved', 'rejected', 'expired'
);

-- LFPIORPI risk classification
CREATE TYPE aml_risk_level AS ENUM ('low', 'medium', 'high');

-- Payment rail
CREATE TYPE payment_rail AS ENUM ('spei', 'codi', 'dimo');

-- Transfer status
CREATE TYPE transfer_status AS ENUM (
    'initiated', 'pending', 'completed', 'failed', 'returned'
);

-- ARCO request type
CREATE TYPE arco_request_type AS ENUM (
    'access', 'rectification', 'cancellation', 'opposition'
);

-- ARCO request status
CREATE TYPE arco_status AS ENUM (
    'received', 'in_progress', 'completed', 'rejected', 'expired'
);

-- Consent method
-- CREATE TYPE consent_method AS ENUM (
--     'electronic_signature', 'biometric', 'click_through', 'in_person'
-- );

-- Device status
CREATE TYPE device_status AS ENUM ('active', 'suspended', 'revoked');
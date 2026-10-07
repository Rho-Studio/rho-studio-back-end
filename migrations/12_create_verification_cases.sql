-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        12_create_verification_cases.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      KYC case lifecycle wrapper. One open case per user at a time. This
--      table tracks case status only; KYC evidence (documents, biometrics,
--      RENAPO/INE references) lives in kyc_verifications (migration 5).
--
--      Tables:
--          - customer_verification_cases  Case per user with status
--                                         (pending, manual_review, verified,
--                                         rejected, cancelled).
--
--      Depends on:
--          - migration 1: pgcrypto (gen_random_uuid)
--          - migration 10: users(id) FK
--
--      Design notes:
--          - Unique partial index idx_customer_verification_cases_one_open_per_user
--            enforces at most one open case (pending or manual_review) per
--            user. Closed cases (verified/rejected/cancelled) do not count.
--          - User-scoped, not customer-scoped. A user can begin verification
--            before a customers row exists.
--
--      Known gaps (tracked in DataModel.md):
--          - routes/onboarding.rs handlers exist but are not registered in
--            build_app. The route is missing.
--          - No FK between customer_verification_cases and kyc_verifications.
--            Case status and KYC evidence live in separate unlinked models.
--          - No updated_at trigger; the handler must set it explicitly.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================
CREATE TABLE customer_verification_cases (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    status      TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'manual_review', 'verified', 'rejected', 'cancelled')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_customer_verification_cases_user_created
    ON customer_verification_cases (user_id, created_at DESC, id DESC);

CREATE UNIQUE INDEX idx_customer_verification_cases_one_open_per_user
    ON customer_verification_cases (user_id)
    WHERE status IN ('pending', 'manual_review');

-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        2_core_domain.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Creates the six aggregate tables representing the current state of the
--      financial domain. The event store (migration 3) is the source of truth;
--      these tables are projections maintained by the application layer during
--      aggregate save operations.
--
--      Tables:
--          - customers               Financial identity anchor. One row per
--                                    natural person or legal entity. Holds
--                                    RFC, CURP, contact, address, KYC level,
--                                    privacy acceptance, PEP/sanctions state.
--          - accounts                Demand-deposit containers. Owns balance
--                                    and held_amount; available is derived.
--          - account_holds           Reserved funds, referenced optionally by
--                                    a soft reference_id (not a FK).
--          - loans                   Issued-loan records post-disbursement.
--                                    Rate stored as basis points (integer).
--          - amortization_schedules  Per-installment breakdown, one row per
--                                    payment, unique per (loan, installment).
--          - loan_applications       Pre-approval requests. Not FK-linked to
--                                    loans; conversion is application-level.
--
--      Depends on migration 1 for:
--          - pgcrypto                    (gen_random_uuid for every PK)
--          - enum kyc_level              (customers.kyc_level)
--          - enum account_status         (accounts.status)
--          - enum loan_status            (loans.status)
--          - enum application_status     (loan_applications.status)
--
--      Design notes:
--          - Every aggregate root carries `version BIGINT` for optimistic
--            concurrency. account_holds is the exception — holds are
--            append-only; only released_at changes.
--          - Money columns use NUMERIC(18,2) for MXN. UDI values use
--            NUMERIC(12,6). Never introduce FLOAT or REAL for money.
--          - Regulatory identifiers (RFC, CURP, CLABE, CAT, UDI) are kept
--            verbatim in Spanish — no English translation — so schema
--            fields map 1:1 to CNBV / Banxico correspondence.
--          - Format validation is enforced by CHECK regex, not application
--            logic, so bulk imports and admin tools are also protected.
--          - Partial indexes cover hot subsets (active holds, active loans
--            by maturity). Query patterns are visible in the DDL.
--
--      Known gaps (tracked in DataModel.md):
--          - balance >= held_amount is not enforced at the DDL level.
--          - CLABE check digit is not validated; format only.
--          - amortization_schedules.total_payment is redundant and
--            unconstrained.
--          - loan_applications has no FK to the resulting loan.
--          - account_holds.reference_id is a soft pointer, not a FK.
--          - No trigger auto-touches updated_at on any table.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================
CREATE TABLE customers (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rfc                 VARCHAR(13) NOT NULL,
    curp                VARCHAR(18),
    first_name          VARCHAR(100) NOT NULL,
    last_name           VARCHAR(100) NOT NULL,
    second_last_name    VARCHAR(100),
    email               VARCHAR(255) NOT NULL,
    phone               VARCHAR(20) NOT NULL,
    date_of_birth       DATE NOT NULL,
    kyc_level           kyc_level NOT NULL DEFAULT 'nivel_1',
    kyc_verified_at     TIMESTAMPTZ,
    address_line        TEXT,
    address_city        VARCHAR(100),
    address_state       VARCHAR(100),
    address_postal_code VARCHAR(10),
    address_country     VARCHAR(3) DEFAULT 'MEX',
    -- LFPDPPP
    privacy_notice_version VARCHAR(20),
    privacy_notice_accepted_at TIMESTAMPTZ,
    -- PEP / sanctions
    pep_status          BOOLEAN NOT NULL DEFAULT FALSE,
    sanctions_checked_at TIMESTAMPTZ,
    -- Aggregate version for optimistic concurrency
    version             BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT customers_rfc_unique UNIQUE (rfc),
    CONSTRAINT customers_curp_unique UNIQUE (curp),
    CONSTRAINT customers_email_unique UNIQUE (email),
    CONSTRAINT customers_rfc_format CHECK (rfc ~ '^[A-ZÑ&]{3,4}[0-9]{6}[A-Z0-9]{3}$'),
    CONSTRAINT customers_curp_format CHECK (
        curp IS NULL OR curp ~ '^[A-Z]{4}[0-9]{6}[HM][A-Z]{5}[0-9]{2}$'
    )
);

CREATE INDEX idx_customers_rfc ON customers (rfc);
CREATE INDEX idx_customers_kyc_level ON customers (kyc_level);

-- ---------------------------------------------------------------------------
-- Accounts
-- ---------------------------------------------------------------------------
CREATE TABLE accounts (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_number      VARCHAR(18) NOT NULL,
    clabe               VARCHAR(18) NOT NULL,
    customer_id         UUID NOT NULL REFERENCES customers(id),
    account_type        VARCHAR(30) NOT NULL DEFAULT 'demand_deposit',
    currency            CHAR(3) NOT NULL DEFAULT 'MXN',
    balance             NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    held_amount         NUMERIC(18,2) NOT NULL DEFAULT 0.00,
    status              account_status NOT NULL DEFAULT 'active',
    opened_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at           TIMESTAMPTZ,
    version             BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT accounts_account_number_unique UNIQUE (account_number),
    CONSTRAINT accounts_clabe_unique UNIQUE (clabe),
    CONSTRAINT accounts_balance_non_negative CHECK (balance >= 0),
    CONSTRAINT accounts_held_non_negative CHECK (held_amount >= 0),
    CONSTRAINT accounts_clabe_format CHECK (clabe ~ '^[0-9]{18}$')
);

CREATE INDEX idx_accounts_customer ON accounts (customer_id);
CREATE INDEX idx_accounts_status ON accounts (status);
CREATE INDEX idx_accounts_clabe ON accounts (clabe);

-- ---------------------------------------------------------------------------
-- Account Holds
-- ---------------------------------------------------------------------------
CREATE TABLE account_holds (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id),
    amount              NUMERIC(18,2) NOT NULL,
    reason              VARCHAR(100) NOT NULL,
    reference_id        UUID,           -- links to loan, transfer, etc.
    placed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at         TIMESTAMPTZ,
    expires_at          TIMESTAMPTZ,

    CONSTRAINT account_holds_amount_positive CHECK (amount > 0)
);

CREATE INDEX idx_holds_account ON account_holds (account_id) WHERE released_at IS NULL;

-- ---------------------------------------------------------------------------
-- Loans
-- ---------------------------------------------------------------------------
CREATE TABLE loans (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    loan_number             VARCHAR(30) NOT NULL,
    customer_id             UUID NOT NULL REFERENCES customers(id),
    disbursement_account_id UUID REFERENCES accounts(id),
    principal_amount        NUMERIC(18,2) NOT NULL,
    outstanding_principal   NUMERIC(18,2) NOT NULL,
    interest_rate_bps       INTEGER NOT NULL,   -- basis points (e.g., 1250 = 12.50%)
    term_months             INTEGER NOT NULL,
    currency                CHAR(3) NOT NULL DEFAULT 'MXN',
    status                  loan_status NOT NULL DEFAULT 'pending_disbursement',
    -- Regulatory
    udi_value_at_origination NUMERIC(12,6),
    cat_average             NUMERIC(8,4),       -- Costo Anual Total (CAT)
    -- Dates
    approved_at             TIMESTAMPTZ,
    disbursed_at            TIMESTAMPTZ,
    maturity_date           DATE,
    paid_off_at             TIMESTAMPTZ,
    -- Aggregate
    version                 BIGINT NOT NULL DEFAULT 0,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT loans_loan_number_unique UNIQUE (loan_number),
    CONSTRAINT loans_principal_positive CHECK (principal_amount > 0),
    CONSTRAINT loans_outstanding_non_negative CHECK (outstanding_principal >= 0),
    CONSTRAINT loans_term_positive CHECK (term_months > 0),
    CONSTRAINT loans_rate_valid CHECK (interest_rate_bps BETWEEN 0 AND 10000)
);

CREATE INDEX idx_loans_customer ON loans (customer_id);
CREATE INDEX idx_loans_status ON loans (status);
CREATE INDEX idx_loans_maturity ON loans (maturity_date) WHERE status = 'active';

-- ---------------------------------------------------------------------------
-- Amortization Schedule
-- ---------------------------------------------------------------------------
CREATE TABLE amortization_schedules (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    loan_id             UUID NOT NULL REFERENCES loans(id),
    installment_number  INTEGER NOT NULL,
    due_date            DATE NOT NULL,
    principal_portion   NUMERIC(18,2) NOT NULL,
    interest_portion    NUMERIC(18,2) NOT NULL,
    total_payment       NUMERIC(18,2) NOT NULL,
    remaining_balance   NUMERIC(18,2) NOT NULL,
    paid_at             TIMESTAMPTZ,
    paid_amount         NUMERIC(18,2),
    version             BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT amortization_unique UNIQUE (loan_id, installment_number),
    CONSTRAINT amortization_installment_positive CHECK (installment_number > 0)
);

CREATE INDEX idx_amortization_loan_due ON amortization_schedules (loan_id, due_date);

-- ---------------------------------------------------------------------------
-- Loan Applications
-- ---------------------------------------------------------------------------
CREATE TABLE loan_applications (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL REFERENCES customers(id),
    requested_amount    NUMERIC(18,2) NOT NULL,
    requested_term      INTEGER NOT NULL,
    purpose             TEXT,
    status              application_status NOT NULL DEFAULT 'submitted',
    credit_score        INTEGER,
    decision_reason     TEXT,
    decided_at          TIMESTAMPTZ,
    version             BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT applications_amount_positive CHECK (requested_amount > 0),
    CONSTRAINT applications_term_positive CHECK (requested_term > 0)
);

CREATE INDEX idx_applications_customer ON loan_applications (customer_id);
CREATE INDEX idx_applications_status ON loan_applications (status);
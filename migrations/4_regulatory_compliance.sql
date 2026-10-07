-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        4_regulatory_compliance.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Mexican regulatory tables supporting LFPIORPI (AML), LFPDPPP (privacy),
--      and CNBV (supervision) reporting obligations. All schema-only — no
--      handler reads or writes these tables yet.
--
--      Tables:
--          - udi_values               Daily UDI value from Banxico SIE
--                                     (series SP68257). Every regulatory
--                                     threshold in the schema is denominated
--                                     in UDIs; this table lets past MXN
--                                     amounts be recomputed historically.
--          - aml_risk_profiles        One row per customer. LFPIORPI risk
--                                     classification (low/medium/high) with
--                                     risk factors, PEP status, sanctions
--                                     match, and next re-assessment date.
--          - uif_notices              UIF avisos filed when thresholds are
--                                     exceeded. Carries the generated XML
--                                     per SAT SPPLD schema.
--          - regulatory_capital       CNBV capital adequacy snapshots per
--                                     reporting date. Distinguishes IFPE /
--                                     IFC / SOFIPO entity types.
--          - cnbv_report_submissions  CNBV SITI filings (R01, R04, R10, ...).
--                                     Unique per (report_series, report_period).
--
--      Depends on:
--          - migration 1: pgcrypto (gen_random_uuid), aml_risk_level enum
--          - migration 2: customers(id) FK
--          - migration 3: soft reference to domain_events (no FK constraint,
--            because domain_events is partitioned)
--
--      Design notes:
--          - Regulatory vocabulary kept verbatim in Spanish (UDI, UMA,
--            LFPIORPI, UIF, CNBV, SITI, IFPE, SOFIPO) so schema fields map
--            1:1 to regulator correspondence.
--          - Money columns use NUMERIC(18,2) for MXN; UDI uses NUMERIC(12,6)
--            (Banxico publishes six-decimal precision). Never FLOAT or REAL.
--          - Re-assessment cadence baked into defaults:
--            aml_risk_profiles.next_assessment_at defaults to +6 months
--            per LFPIORPI expectation.
--          - Partial indexes cover hot status queues. Submitted UIF notices
--            drop out of idx_uif_notices_status, keeping it small.
--          - regulatory_capital stores the UDI value used at report time,
--            not just the derived amount, so reports are self-contained and
--            reproducible years later.
--          - uif_notices.event_id deliberately has no FK: partitioned tables
--            require the partition key in every unique constraint, and
--            event_id alone doesn't carry it.
--
--      Known gaps (tracked in DataModel.md):
--          - No scheduled job fetches UDI values, generates UIF notices,
--            or files CNBV reports. udi_values holds only the seed row.
--          - No amendment flow for cnbv_report_submissions; the UNIQUE
--            constraint blocks re-filing without a schema change.
--          - uif_notices.event_id is a soft pointer; nothing prevents
--            dangling references.
--          - notice_type and vulnerable_activity are VARCHAR, not enums;
--            typos aren't caught at the DDL level (deliberate — the LFPIORPI
--            activity catalogue is regulator-updated).
--          - uif_notices has no updated_at; status transitions (pending →
--            submitted → failed) aren't timestamped.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- UDI Cache — Unidad de Inversión daily value from Banxico.
-- Every regulatory threshold is denominated in UDIs. This table caches the
-- daily value fetched from Banxico's SIE API (series SP68257).
-- ---------------------------------------------------------------------------
CREATE TABLE udi_values (
    date                DATE PRIMARY KEY,
    value_mxn           NUMERIC(12,6) NOT NULL,
    source              VARCHAR(20) NOT NULL DEFAULT 'banxico_sie',
    fetched_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT udi_value_positive CHECK (value_mxn > 0)
);

-- Seed with a placeholder; the application fetches the real value daily.
INSERT INTO udi_values (date, value_mxn)
VALUES (CURRENT_DATE, 8.810000)
ON CONFLICT (date) DO NOTHING;

-- ---------------------------------------------------------------------------
-- LFPIORPI — AML Risk Profiles
-- Risk-based approach: classify each customer into low, medium, or high.
-- Re-evaluate at least every six months, more frequently as risk increases.
-- ---------------------------------------------------------------------------
CREATE TABLE aml_risk_profiles (
    customer_id         UUID PRIMARY KEY REFERENCES customers(id),
    risk_level          aml_risk_level NOT NULL DEFAULT 'low',
    risk_factors        JSONB NOT NULL DEFAULT '[]'::jsonb,
    pep_status          BOOLEAN NOT NULL DEFAULT FALSE,
    sanctions_match     BOOLEAN NOT NULL DEFAULT FALSE,
    country_risk        VARCHAR(10) NOT NULL DEFAULT 'low',
    last_assessed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_assessment_at  TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '6 months',
    assessed_by         VARCHAR(100),
    notes               TEXT,
    version             BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_aml_risk_next_assessment ON aml_risk_profiles (next_assessment_at);

-- ---------------------------------------------------------------------------
-- LFPIORPI — UIF Notices (Avisos)
-- Generated when a transaction exceeds the reporting threshold (645 UMA).
-- The XML payload follows the SAT's SPPLD schema.
-- ---------------------------------------------------------------------------
CREATE TABLE uif_notices (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL REFERENCES customers(id),
    event_id            UUID NOT NULL,          -- FK to domain_events (no constraint due to partitioning)
    notice_type         VARCHAR(20) NOT NULL,    -- relevante | inusual | interna
    vulnerable_activity VARCHAR(4) NOT NULL,
    amount_mxn          NUMERIC(18,2) NOT NULL,
    amount_uma          NUMERIC(18,4),
    operation_date      DATE NOT NULL,
    detection_date      DATE NOT NULL,
    xml_payload         TEXT,                    -- generated XML for SPPLD
    submitted_at        TIMESTAMPTZ,
    submission_ack      VARCHAR(100),            -- SAT acknowledgment reference
    submission_status   VARCHAR(20) NOT NULL DEFAULT 'pending',
    retry_count         INTEGER NOT NULL DEFAULT 0,
    last_error          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uif_notice_amount_positive CHECK (amount_mxn > 0)
);

CREATE INDEX idx_uif_notices_customer ON uif_notices (customer_id, operation_date);
CREATE INDEX idx_uif_notices_status ON uif_notices (submission_status)
    WHERE submission_status != 'submitted';

-- ---------------------------------------------------------------------------
-- CNBV — Regulatory Capital Tracking
-- IFPE minimum capital: 500,000–700,000 UDIs depending on business model.
-- SOFIPO tiered capital. This table tracks the institution's capital position.
-- ---------------------------------------------------------------------------
CREATE TABLE regulatory_capital (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reporting_date      DATE NOT NULL,
    capital_neto_mxn    NUMERIC(18,2) NOT NULL,
    capital_requerido_mxn NUMERIC(18,2) NOT NULL,
    capital_requerido_udis NUMERIC(18,6) NOT NULL,
    udi_value           NUMERIC(12,6) NOT NULL,
    capital_ratio       NUMERIC(8,4) NOT NULL,
    entity_type         VARCHAR(10) NOT NULL DEFAULT 'ifpe',  -- ifpe | ifc | sofipo
    entity_level        VARCHAR(10),                          -- for sofipo tiers
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT regulatory_capital_unique UNIQUE (reporting_date, entity_type),
    CONSTRAINT capital_neto_positive CHECK (capital_neto_mxn > 0)
);

-- ---------------------------------------------------------------------------
-- CNBV — Regulatory Report Submissions
-- Tracks every report filed with the CNBV via SITI.
-- Report series: R01 (catálogo mínimo), R04 (cartera de crédito), etc.
-- ---------------------------------------------------------------------------
CREATE TABLE cnbv_report_submissions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    report_series       VARCHAR(10) NOT NULL,     -- R01, R04, R10, etc.
    report_period       DATE NOT NULL,            -- the period being reported
    xml_payload         TEXT,
    submitted_at        TIMESTAMPTZ,
    submission_ack      VARCHAR(100),
    submission_status   VARCHAR(20) NOT NULL DEFAULT 'pending',
    error_details       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT cnbv_report_unique UNIQUE (report_series, report_period)
);
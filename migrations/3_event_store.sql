-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        3_event_store.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Append-only event log for the financial domain. Every aggregate state
--      change is recorded here first; migration 2's aggregate tables are
--      projections of this log. Also declares the optimistic-concurrency
--      enforcer, per-aggregate snapshots, and the transactional outbox.
--
--      Tables:
--          - domain_events           The event log. Partitioned monthly by
--                                    occurred_at. Composite PK (id, occurred_at)
--                                    because Postgres requires the partition
--                                    key in every unique constraint.
--          - aggregate_versions      One row per aggregate. The optimistic-
--                                    concurrency enforcer. Updated in the same
--                                    transaction that appends events. Not
--                                    partitioned — one row per aggregate,
--                                    independent of event volume.
--          - aggregate_snapshots     Serialized state at a given version, so
--                                    replay does not need to process the full
--                                    history. Overwritten, not versioned —
--                                    snapshots are a cache, not truth.
--          - outbox_events           Transactional outbox for reliable
--                                    external publishing. Rows are inserted
--                                    in the same transaction as domain_events;
--                                    a separate publisher drains and marks
--                                    them published.
--
--      Depends on migration 1 for:
--          - pgcrypto                    (gen_random_uuid for outbox_events.id)
--
--      Partitions:
--          Twelve monthly partitions are created up front (2026-09 through
--          2027-08) plus domain_events_default. The default partition is a
--          safety net, not a strategy — a scheduled job must create new
--          partitions ahead of time. Monthly granularity is chosen so that
--          retention is a partition-detach operation, not a mass delete.
--
--      Design notes:
--          - Append-only. No UPDATE, no DELETE on domain_events. Corrections
--            are new compensating events, never edits to history.
--          - Partitioned by occurred_at (when the event happened), not by
--            created_at (when the row was written). Batch imports of historical
--            data land in the correct month's partition.
--          - Regulatory columns (rfc, curp, amount_mxn, amount_udis, udi_value,
--            vulnerable_activity, regulatory_report_id) are denormalized onto
--            the event. CNBV/UIF reports are answerable from the event store
--            alone, without joining to current-state tables that reflect today
--            rather than the reporting period.
--          - Money columns use NUMERIC(18,2) for MXN. UDI values use
--            NUMERIC(12,6). Never FLOAT or REAL.
--
--      Known gaps (tracked in DataModel.md):
--          - No partition-management job; partitions must be created manually.
--          - No application code writes to domain_events yet. Schema only.
--          - No projector consumes events to populate migration 7's read models.
--          - No outbox publisher; rows accumulate until a drainer exists.
--          - No snapshot writer; replay is full-history until that exists.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

CREATE TABLE domain_events (
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    aggregate_id        UUID NOT NULL,
    aggregate_type      VARCHAR(50) NOT NULL,
    event_type          VARCHAR(100) NOT NULL,
    payload             JSONB NOT NULL,
    version             BIGINT NOT NULL,
    occurred_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    correlation_id      UUID NOT NULL,
    causation_id        UUID,
    device_id           UUID,
    app_version         VARCHAR(20),
    platform            VARCHAR(10),
    ip_address          INET,
    geolocation         JSONB,
    rfc                 VARCHAR(13),
    curp                VARCHAR(18),
    amount_mxn          NUMERIC(18,2),
    amount_udis         NUMERIC(18,6),
    udi_value           NUMERIC(12,6),
    vulnerable_activity VARCHAR(4),
    regulatory_report_id UUID,
    PRIMARY KEY (id, occurred_at)
) PARTITION BY RANGE (occurred_at);

-- ---------------------------------------------------------------------------
-- The actual optimistic-concurrency enforcer.
-- Not partitioned. One row per aggregate. Updated in the same transaction
-- that appends events. If rows_affected = 0, the version is stale.
-- ---------------------------------------------------------------------------
CREATE TABLE aggregate_versions (
    aggregate_id    UUID NOT NULL,
    aggregate_type  VARCHAR(50) NOT NULL,
    current_version BIGINT NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (aggregate_id, aggregate_type)
);

-- ---------------------------------------------------------------------------
-- Monthly partitions. Create 12 months ahead at deploy time.
-- The default partition is a safety net, not a strategy.
-- ---------------------------------------------------------------------------
CREATE TABLE domain_events_2026_09 PARTITION OF domain_events
    FOR VALUES FROM ('2026-09-01') TO ('2026-10-01');
CREATE TABLE domain_events_2026_10 PARTITION OF domain_events
    FOR VALUES FROM ('2026-10-01') TO ('2026-11-01');
CREATE TABLE domain_events_2026_11 PARTITION OF domain_events
    FOR VALUES FROM ('2026-11-01') TO ('2026-12-01');
CREATE TABLE domain_events_2026_12 PARTITION OF domain_events
    FOR VALUES FROM ('2026-12-01') TO ('2027-01-01');
CREATE TABLE domain_events_2027_01 PARTITION OF domain_events
    FOR VALUES FROM ('2027-01-01') TO ('2027-02-01');
CREATE TABLE domain_events_2027_02 PARTITION OF domain_events
    FOR VALUES FROM ('2027-02-01') TO ('2027-03-01');
CREATE TABLE domain_events_2027_03 PARTITION OF domain_events
    FOR VALUES FROM ('2027-03-01') TO ('2027-04-01');
CREATE TABLE domain_events_2027_04 PARTITION OF domain_events
    FOR VALUES FROM ('2027-04-01') TO ('2027-05-01');
CREATE TABLE domain_events_2027_05 PARTITION OF domain_events
    FOR VALUES FROM ('2027-05-01') TO ('2027-06-01');
CREATE TABLE domain_events_2027_06 PARTITION OF domain_events
    FOR VALUES FROM ('2027-06-01') TO ('2027-07-01');
CREATE TABLE domain_events_2027_07 PARTITION OF domain_events
    FOR VALUES FROM ('2027-07-01') TO ('2027-08-01');
CREATE TABLE domain_events_2027_08 PARTITION OF domain_events
    FOR VALUES FROM ('2027-08-01') TO ('2027-09-01');

CREATE TABLE domain_events_default PARTITION OF domain_events DEFAULT;

-- ---------------------------------------------------------------------------
-- Indexes
-- ---------------------------------------------------------------------------
CREATE INDEX idx_events_aggregate ON domain_events (aggregate_id, version);
CREATE INDEX idx_events_type_time ON domain_events (event_type, occurred_at);
CREATE INDEX idx_events_correlation ON domain_events (correlation_id);
CREATE INDEX idx_events_rfc_time ON domain_events (rfc, occurred_at);
CREATE INDEX idx_events_activity_time ON domain_events (vulnerable_activity, occurred_at);
CREATE INDEX idx_events_amount_time ON domain_events (amount_mxn, occurred_at);
CREATE INDEX idx_events_device ON domain_events (device_id, occurred_at);

-- ---------------------------------------------------------------------------
-- Snapshots
-- ---------------------------------------------------------------------------
CREATE TABLE aggregate_snapshots (
    aggregate_id        UUID NOT NULL,
    aggregate_type      VARCHAR(50) NOT NULL,
    version             BIGINT NOT NULL,
    state               JSONB NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (aggregate_id, aggregate_type)
);

-- ---------------------------------------------------------------------------
-- Outbox
-- ---------------------------------------------------------------------------
CREATE TABLE outbox_events (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    aggregate_id        UUID NOT NULL,
    event_type          VARCHAR(100) NOT NULL,
    payload             JSONB NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at        TIMESTAMPTZ,
    retry_count         INTEGER NOT NULL DEFAULT 0,
    last_error          TEXT
);

CREATE INDEX idx_outbox_unpublished ON outbox_events (created_at)
    WHERE published_at IS NULL;
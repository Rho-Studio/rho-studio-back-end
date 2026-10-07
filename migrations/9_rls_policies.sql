-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        9_rls_policies.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Row-Level Security policies on customer-scoped tables. Even a buggy
--      application cannot leak one customer's data to another. Tenant context
--      is set per request via:
--          SET LOCAL app.current_customer_id = '<uuid>';
--
--      Secured tables:
--          - customers
--          - accounts
--          - loans
--          - transaction_history
--          - sync_changes
--          - arco_requests
--
--      Depends on:
--          - migrations 2, 5, 7: the tables being secured
--
--      Design notes:
--          - FOR SELECT only. Postgres default: no policy = denied. Writes
--            are blocked by default unless a policy is added or the service
--            account setting is set.
--          - current_setting(..., TRUE) uses missing_ok = true so an unset
--            variable returns NULL rather than erroring; the NULL comparison
--            filters the row out — fail closed.
--          - is_service_account bypass. Connections that set
--            app.is_service_account = 'true' see all rows. Migrations, admin
--            tools, and background jobs use this. Application connections
--            never do.
--          - Uniform policy shape across every table: same predicate pattern
--            for both tenant and service-account axes.
--          - RLS as defense in depth. Application-level ownership checks
--            also exist; RLS catches the case where one is forgotten.
--          - domain_events deliberately excluded. The event store is
--            internal; customers never query it directly. Regulatory queries
--            use the service account role. Enabling RLS on the partitioned
--            parent would complicate partition-level queries.
--          - consent_records block commented out: the table was removed.
--            Retained as history; uncommenting requires the table to exist.
--
--      Known gaps (tracked in DataModel.md):
--          - No handler sets app.current_customer_id. Every read from a
--            secured table returns zero rows until the middleware is wired.
--          - Only SELECT policies. Writes need their own policies.
--          - users / refresh_tokens not covered by RLS.
--          - consent_notices / customer_consent_events not covered.
--          - kyc_verifications, registered_devices, device_tokens not
--            covered despite being customer-scoped.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- Customers — a customer can see only their own record (via the API layer,
-- not directly). Service accounts bypass RLS.
-- ---------------------------------------------------------------------------
ALTER TABLE customers ENABLE ROW LEVEL SECURITY;

CREATE POLICY customers_self_access ON customers
    FOR SELECT
    USING (
        id = current_setting('app.current_customer_id', TRUE)::UUID
        OR current_setting('app.is_service_account', TRUE) = 'true'
    );

-- ---------------------------------------------------------------------------
-- Accounts
-- ---------------------------------------------------------------------------
ALTER TABLE accounts ENABLE ROW LEVEL SECURITY;

CREATE POLICY accounts_self_access ON accounts
    FOR SELECT
    USING (
        customer_id = current_setting('app.current_customer_id', TRUE)::UUID
        OR current_setting('app.is_service_account', TRUE) = 'true'
    );

-- ---------------------------------------------------------------------------
-- Loans
-- ---------------------------------------------------------------------------
ALTER TABLE loans ENABLE ROW LEVEL SECURITY;

CREATE POLICY loans_self_access ON loans
    FOR SELECT
    USING (
        customer_id = current_setting('app.current_customer_id', TRUE)::UUID
        OR current_setting('app.is_service_account', TRUE) = 'true'
    );

-- ---------------------------------------------------------------------------
-- Transaction History
-- ---------------------------------------------------------------------------
ALTER TABLE transaction_history ENABLE ROW LEVEL SECURITY;

CREATE POLICY txn_history_self_access ON transaction_history
    FOR SELECT
    USING (
        customer_id = current_setting('app.current_customer_id', TRUE)::UUID
        OR current_setting('app.is_service_account', TRUE) = 'true'
    );

-- ---------------------------------------------------------------------------
-- Sync Changes
-- ---------------------------------------------------------------------------
ALTER TABLE sync_changes ENABLE ROW LEVEL SECURITY;

CREATE POLICY sync_changes_self_access ON sync_changes
    FOR SELECT
    USING (
        customer_id = current_setting('app.current_customer_id', TRUE)::UUID
        OR current_setting('app.is_service_account', TRUE) = 'true'
    );

-- ---------------------------------------------------------------------------
-- Consents and ARCO Requests
-- Consent evidence for the current system lives in `customer_consent_events`
-- (user-scoped, migration 11). The legacy `consent_records` table was removed.
-- ---------------------------------------------------------------------------

-- ALTER TABLE consent_records ENABLE ROW LEVEL SECURITY;

-- CREATE POLICY consent_self_access ON consent_records
--     FOR SELECT
--     USING (
--         customer_id = current_setting('app.current_customer_id', TRUE)::UUID
--         OR current_setting('app.is_service_account', TRUE) = 'true'
--     );

ALTER TABLE arco_requests ENABLE ROW LEVEL SECURITY;

CREATE POLICY arco_self_access ON arco_requests
    FOR SELECT
    USING (
        customer_id = current_setting('app.current_customer_id', TRUE)::UUID
        OR current_setting('app.is_service_account', TRUE) = 'true'
    );

-- ---------------------------------------------------------------------------
-- Note on the event store: RLS is NOT enabled on domain_events.
-- The event store is internal. It is never queried directly by customers.
-- Only the application's read-model projections are customer-facing.
-- Regulatory queries against the event store use the service account role.
-- ---------------------------------------------------------------------------
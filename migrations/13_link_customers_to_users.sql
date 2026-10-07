-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        13_link_customers_to_users.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Links the financial customer profile to its authenticated login
--      identity. NULL is permitted for existing / unlinked customer records
--      and for login-only users who have not completed onboarding.
--
--      Changes:
--          - Adds customers.user_id UUID FK to users(id) ON DELETE RESTRICT.
--          - Unique partial index customers_user_id_unique so a user can
--            own at most one customer row, and NULLs are not deduplicated.
--          - Column comment documents the linking contract.
--
--      Depends on:
--          - migration 2: customers(id)
--          - migration 10: users(id)
--
--      Design notes:
--          - Never infer this association from matching email, RFC, or other
--            profile data. Link only after verified identity proof. This is
--            a compliance contract, not just a schema rule — LFPDPPP requires
--            the linkage to be evidence-based.
--          - ON DELETE RESTRICT. A user with a linked customer row cannot be
--            deleted; the customer must be unlinked first.
--          - Partial unique index (WHERE user_id IS NOT NULL) means many
--            unlinked customers can coexist, but each linked user maps to
--            exactly one customer.
--
--      Known gaps (tracked in DataModel.md):
--          - No handler performs the linkage. The onboarding flow that would
--            associate a user with a customer does not exist.
--          - No backfill for existing customer rows. They remain unlinked
--            until an explicit process assigns them.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

ALTER TABLE customers
    ADD COLUMN user_id UUID REFERENCES users(id) ON DELETE RESTRICT;

CREATE UNIQUE INDEX customers_user_id_unique
    ON customers (user_id)
    WHERE user_id IS NOT NULL;

COMMENT ON COLUMN customers.user_id IS
    'Optional authenticated owner; unique when set. Link only after verified identity proof.';
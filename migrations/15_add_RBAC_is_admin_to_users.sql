-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        15_add_RBAC_is_admin_to_users.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-11-03
-- ============================================================================
-- Migration 14: Add RBAC (is_admin) to users
-- ============================================================================
-- Adds a boolean flag to distinguish compliance officers/admins from standard
-- customers. This is used to protect sensitive endpoints like publishing
-- Privacy Notices.
--
-- In a future phase, this can be normalized into a `roles` and `user_roles` 
-- table structure, but for the MVP, a boolean is performant and sufficient.
-- ============================================================================

ALTER TABLE users 
ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT FALSE;

-- Create a partial index to quickly query for admins (e.g., for notification blasts)
CREATE INDEX idx_users_is_admin ON users (is_admin) WHERE is_admin = TRUE;

-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        10_create_identity_sessions.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Identity and refresh-session tables used by the current authentication
--      API. This is the only migration whose tables are exercised end-to-end
--      at the HTTP layer today.
--
--      Tables:
--          - users             Authentication anchor. Unique case-insensitive
--                              email index; Argon2id password_hash. Both
--                              is_active and email_verified gate login.
--          - refresh_tokens    SHA-256 hashed refresh tokens grouped by
--                              token_family_id. Rotation revokes the previous
--                              row and inserts a new one in the same family;
--                              reuse of a revoked token triggers family-wide
--                              revocation in the handler.
--
--      Depends on:
--          - migration 1: pgcrypto (gen_random_uuid)
--
--      Design notes:
--          - users.email is stored as-given; uniqueness is enforced on
--            lower(email) via a unique index. Login queries
--            WHERE lower(email) = lower($1) and relies on that index.
--          - refresh_tokens.id equals the JWT jti claim. The handler generates
--            the UUID and embeds it in the signed token before insert. No DB
--            default — the value must match.
--          - FK user_id ON DELETE RESTRICT on both tables. A user with live
--            sessions cannot be deleted; explicit revocation must precede
--            deletion.
--          - Partial index refresh_tokens_family_active_idx
--            (token_family_id, expires_at) WHERE revoked_at IS NULL powers the
--            per-request session-liveness check in authenticated_user_id.
--          - Money: no monetary columns in this migration.
--
--      Known gaps (tracked in DataModel.md):
--          - signup handler sets email_verified = true for dev convenience.
--            Production must flip to false and add a verification flow
--            (future).
--          - No trigger auto-touches users.updated_at; the first UPDATE
--            handler must set it explicitly.
--          - No RLS on users / refresh_tokens. Auth tables rely on the
--            application layer always filtering by id or token_hash.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email           VARCHAR(320) NOT NULL,
    display_name    VARCHAR(120),
    password_hash   TEXT NOT NULL,
    email_verified  BOOLEAN NOT NULL DEFAULT FALSE,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT users_email_not_empty CHECK (length(btrim(email)) > 0),
    CONSTRAINT users_display_name_length CHECK (
        display_name IS NULL OR length(display_name) <= 120
    ),
    CONSTRAINT users_password_hash_not_empty CHECK (length(password_hash) > 0)
);

CREATE UNIQUE INDEX users_email_lower_unique ON users (lower(email));

CREATE TABLE refresh_tokens (
    id              UUID PRIMARY KEY,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    token_hash      VARCHAR(64) NOT NULL,
    token_family_id UUID NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    revoked_at      TIMESTAMPTZ,
    user_agent      VARCHAR(512),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT refresh_tokens_hash_hex CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT refresh_tokens_expiry_after_creation CHECK (expires_at > created_at),
    CONSTRAINT refresh_tokens_revoked_after_creation CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    )
);

CREATE UNIQUE INDEX refresh_tokens_hash_unique ON refresh_tokens (token_hash);
CREATE INDEX refresh_tokens_family_active_idx
    ON refresh_tokens (token_family_id, expires_at)
    WHERE revoked_at IS NULL;
CREATE INDEX refresh_tokens_user_idx ON refresh_tokens (user_id, created_at DESC);

-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        11_create_consent_registry.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-10-07
-- ============================================================================
-- Description:
--      Versioned notice registry and append-only consent evidence. This is
--      the current consent model, user-scoped and separate from the legacy
--      customer-scoped consent_records table removed in migration 5.
--
--      Tables:
--          - consent_notices          Versioned, locale-scoped notice
--                                     content. Immutable once published.
--          - customer_consent_events  Append-only grant/withdraw evidence
--                                     per (user, notice).
--
--      Depends on:
--          - migration 1: pgcrypto (gen_random_uuid, digest)
--          - migration 10: users(id) FK
--
--      Design notes:
--          - consent_notices is protected by two triggers:
--              protect_consent_notice()  — verifies content_sha256 matches
--                                          content, enforces immutability
--                                          of published/retired rows, and
--                                          sets published_at/retired_at on
--                                          transitions.
--              prevent_consent_notice_delete() — blocks DELETE entirely.
--            Retire, don't delete.
--          - One published notice per (purpose_code, locale), enforced by a
--            partial unique index.
--          - customer_consent_events is append-only: a trigger blocks UPDATE
--            and DELETE. Current state = latest event by
--            (recorded_at DESC, id DESC).
--          - recorded_at defaults to clock_timestamp(), not now(), so events
--            recorded in the same transaction get distinct timestamps.
--
--      Known gaps (tracked in DataModel.md):
--          - content_sha256 must be computed by the writer; nothing prevents
--            a mismatch until the trigger fires.
--          - No consent notice seeder; notices must be inserted manually or
--            by an admin tool.
--
--      Not idempotent. Re-running requires a wiped database.
-- ============================================================================

CREATE TABLE consent_notices (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    purpose_code    VARCHAR(64) NOT NULL,
    locale          VARCHAR(16) NOT NULL,
    version         VARCHAR(64) NOT NULL,
    content         TEXT NOT NULL,
    content_sha256  VARCHAR(64) NOT NULL,
    status          VARCHAR(16) NOT NULL DEFAULT 'draft',
    published_at    TIMESTAMPTZ,
    retired_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT consent_notices_purpose_not_empty CHECK (length(btrim(purpose_code)) > 0),
    CONSTRAINT consent_notices_locale_not_empty CHECK (length(btrim(locale)) > 0),
    CONSTRAINT consent_notices_version_not_empty CHECK (length(btrim(version)) > 0),
    CONSTRAINT consent_notices_content_not_empty CHECK (length(btrim(content)) > 0),
    CONSTRAINT consent_notices_hash_hex CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT consent_notices_status_valid CHECK (status IN ('draft', 'published', 'retired')),
    CONSTRAINT consent_notices_publication_state CHECK (
        (status = 'draft' AND published_at IS NULL AND retired_at IS NULL)
        OR (status = 'published' AND published_at IS NOT NULL AND retired_at IS NULL)
        OR (status = 'retired' AND published_at IS NOT NULL AND retired_at IS NOT NULL)
    ),
    CONSTRAINT consent_notices_version_unique UNIQUE (purpose_code, locale, version)
);

CREATE UNIQUE INDEX consent_notices_one_published_per_purpose_locale
    ON consent_notices (purpose_code, locale)
    WHERE status = 'published';

CREATE FUNCTION protect_consent_notice() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF encode(digest(NEW.content, 'sha256'), 'hex') <> NEW.content_sha256 THEN
        RAISE EXCEPTION 'consent notice content hash does not match content';
    END IF;

    IF TG_OP = 'UPDATE' AND OLD.status IN ('published', 'retired') THEN
        IF ROW(NEW.purpose_code, NEW.locale, NEW.version, NEW.content, NEW.content_sha256)
            IS DISTINCT FROM
           ROW(OLD.purpose_code, OLD.locale, OLD.version, OLD.content, OLD.content_sha256) THEN
            RAISE EXCEPTION 'published consent notice identity and content are immutable';
        END IF;

        IF OLD.status = 'retired' AND NEW.status <> 'retired' THEN
            RAISE EXCEPTION 'retired consent notices cannot be republished';
        END IF;

        IF OLD.status = 'published' AND NEW.status NOT IN ('published', 'retired') THEN
            RAISE EXCEPTION 'published consent notices may only be retired';
        END IF;
    END IF;

    IF TG_OP = 'UPDATE' AND OLD.status = 'draft' AND NEW.status = 'published' THEN
        NEW.published_at := COALESCE(NEW.published_at, now());
        NEW.retired_at := NULL;
    ELSIF TG_OP = 'UPDATE' AND OLD.status = 'published' AND NEW.status = 'retired' THEN
        NEW.retired_at := COALESCE(NEW.retired_at, now());
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER consent_notices_protect_content
    BEFORE INSERT OR UPDATE ON consent_notices
    FOR EACH ROW EXECUTE FUNCTION protect_consent_notice();

CREATE FUNCTION prevent_consent_notice_delete() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'consent notices cannot be deleted; retire the notice instead';
END;
$$;

CREATE TRIGGER consent_notices_no_delete
    BEFORE DELETE ON consent_notices
    FOR EACH ROW EXECUTE FUNCTION prevent_consent_notice_delete();

CREATE TABLE customer_consent_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    notice_id       UUID NOT NULL REFERENCES consent_notices(id) ON DELETE RESTRICT,
    event_type      VARCHAR(16) NOT NULL,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    CONSTRAINT customer_consent_events_type_valid
        CHECK (event_type IN ('granted', 'withdrawn'))
);

CREATE INDEX customer_consent_events_latest_idx
    ON customer_consent_events (user_id, notice_id, recorded_at DESC, id DESC);
CREATE INDEX customer_consent_events_notice_idx
    ON customer_consent_events (notice_id, recorded_at DESC);

CREATE FUNCTION prevent_customer_consent_event_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'customer consent events are append-only';
END;
$$;

CREATE TRIGGER customer_consent_events_no_update_or_delete
    BEFORE UPDATE OR DELETE ON customer_consent_events
    FOR EACH ROW EXECUTE FUNCTION prevent_customer_consent_event_mutation();

CREATE TABLE exercises (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name              TEXT NOT NULL,
    slug              TEXT NOT NULL UNIQUE,
    description       TEXT,
    instructions      TEXT,
    muscle_group      TEXT NOT NULL,
    secondary_muscles TEXT[] NOT NULL DEFAULT '{}',
    equipment         TEXT,
    difficulty        TEXT NOT NULL DEFAULT 'beginner'
                      CHECK (difficulty IN ('beginner', 'intermediate', 'advanced')),
    category          TEXT,
    gif_storage_key   TEXT,
    video_url         TEXT,
    thumbnail_key     TEXT,
    source            TEXT,
    source_id         TEXT,
    is_published      BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_exercises_source_source_id
    ON exercises (source, source_id)
    WHERE source IS NOT NULL AND source_id IS NOT NULL;

CREATE INDEX idx_exercises_muscle_group ON exercises(muscle_group);
CREATE INDEX idx_exercises_equipment    ON exercises(equipment);
CREATE INDEX idx_exercises_difficulty   ON exercises(difficulty);
CREATE INDEX idx_exercises_category     ON exercises(category);
CREATE INDEX idx_exercises_is_published ON exercises(is_published) WHERE is_published = true;

CREATE INDEX idx_exercises_name_trgm ON exercises USING gin (name gin_trgm_ops);

CREATE INDEX idx_exercises_fts
    ON exercises USING gin (
        to_tsvector('english',
            coalesce(name, '') || ' ' || coalesce(description, ''))
    );

CREATE TRIGGER trg_exercises_updated_at
    BEFORE UPDATE ON exercises
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TABLE routines (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    description   TEXT,
    created_by    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    is_public     BOOLEAN NOT NULL DEFAULT false,
    goal          TEXT,
    difficulty    TEXT
                  CHECK (difficulty IN ('beginner', 'intermediate', 'advanced')),
    estimated_duration_minutes INTEGER,
    cover_image_key TEXT,
    tags          TEXT[] NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_routines_created_by ON routines(created_by);
CREATE INDEX idx_routines_is_public  ON routines(is_public) WHERE is_public = true;
CREATE INDEX idx_routines_goal       ON routines(goal);
CREATE INDEX idx_routines_tags       ON routines USING gin (tags);
CREATE INDEX idx_routines_name_trgm  ON routines USING gin (name gin_trgm_ops);

CREATE TRIGGER trg_routines_updated_at
    BEFORE UPDATE ON routines
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE routine_exercises (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    routine_id       UUID NOT NULL REFERENCES routines(id)  ON DELETE CASCADE,
    exercise_id      UUID NOT NULL REFERENCES exercises(id) ON DELETE RESTRICT,
    position         INTEGER NOT NULL,
    sets             INTEGER CHECK (sets IS NULL OR sets > 0),
    reps             INTEGER CHECK (reps IS NULL OR reps > 0),
    weight_kg        NUMERIC(6,2) CHECK (weight_kg IS NULL OR weight_kg >= 0),
    duration_seconds INTEGER CHECK (duration_seconds IS NULL OR duration_seconds > 0),
    rest_seconds     INTEGER NOT NULL DEFAULT 60 CHECK (rest_seconds >= 0),
    notes            TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (routine_id, position)
);

CREATE INDEX idx_routine_exercises_routine_id  ON routine_exercises(routine_id);
CREATE INDEX idx_routine_exercises_exercise_id ON routine_exercises(exercise_id);
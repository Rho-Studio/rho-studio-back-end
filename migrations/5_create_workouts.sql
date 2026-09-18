CREATE TABLE workout_logs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id          UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    routine_id       UUID REFERENCES routines(id) ON DELETE SET NULL,
    name             TEXT,
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at     TIMESTAMPTZ,
    duration_seconds INTEGER CHECK (duration_seconds IS NULL OR duration_seconds >= 0),
    notes            TEXT,
    rating           SMALLINT CHECK (rating IS NULL OR (rating BETWEEN 1 AND 5)),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_workout_logs_user_date      ON workout_logs(user_id, started_at DESC);
CREATE INDEX idx_workout_logs_user_completed ON workout_logs(user_id, completed_at)
    WHERE completed_at IS NOT NULL;
CREATE INDEX idx_workout_logs_routine_id     ON workout_logs(routine_id);

CREATE TRIGGER trg_workout_logs_updated_at
    BEFORE UPDATE ON workout_logs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE workout_log_exercises (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    log_id           UUID NOT NULL REFERENCES workout_logs(id) ON DELETE CASCADE,
    exercise_id      UUID NOT NULL REFERENCES exercises(id)    ON DELETE RESTRICT,
    position         INTEGER NOT NULL,
    sets             INTEGER CHECK (sets IS NULL OR sets > 0),
    reps             INTEGER CHECK (reps IS NULL OR reps > 0),
    weight_kg        NUMERIC(6,2) CHECK (weight_kg IS NULL OR weight_kg >= 0),
    duration_seconds INTEGER CHECK (duration_seconds IS NULL OR duration_seconds >= 0),
    rest_seconds     INTEGER CHECK (rest_seconds IS NULL OR rest_seconds >= 0),
    notes            TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (log_id, position)
);

CREATE INDEX idx_workout_log_exercises_log_id      ON workout_log_exercises(log_id);
CREATE INDEX idx_workout_log_exercises_exercise_id ON workout_log_exercises(exercise_id);
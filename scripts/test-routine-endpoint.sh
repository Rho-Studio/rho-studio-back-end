#!/usr/bin/env bash
set -euo pipefail

base_url="${BASE_URL:-http://127.0.0.1:8080}"
public_routine_id="00000000-0000-0000-0000-000000000902"
private_routine_id="00000000-0000-0000-0000-000000000903"

if ! curl -fsS "$base_url/health" >/dev/null; then
    printf 'backend is not reachable at %s\n' "$base_url" >&2
    printf 'start it in another terminal with: cargo run\n' >&2
    printf 'or set BASE_URL to the running backend URL\n' >&2
    exit 1
fi

docker compose exec -T postgres psql -U rho_studio -d rho_studio -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;

INSERT INTO users (id, email, password_hash, display_name, role, email_verified)
VALUES (
    '00000000-0000-0000-0000-000000000901',
    'api-test@example.com',
    'not-used',
    'API Test Author',
    'user',
    true
)
ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name;

DELETE FROM routines
WHERE id IN (
    '00000000-0000-0000-0000-000000000902',
    '00000000-0000-0000-0000-000000000903'
);

INSERT INTO routines (id, name, description, created_by, is_public)
VALUES
    (
        '00000000-0000-0000-0000-000000000902',
        'API Test Routine',
        'Routine returned by the API test.',
        '00000000-0000-0000-0000-000000000901',
        true
    ),
    (
        '00000000-0000-0000-0000-000000000903',
        'Private API Test Routine',
        'This routine must not be returned.',
        '00000000-0000-0000-0000-000000000901',
        false
    );

DELETE FROM routine_exercises
WHERE routine_id = '00000000-0000-0000-0000-000000000902';

DELETE FROM exercises
WHERE id IN (
    '00000000-0000-0000-0000-000000000904',
    '00000000-0000-0000-0000-000000000905'
);

INSERT INTO routine_exercises (routine_id, exercise_id, position, sets, reps, rest_seconds)
SELECT
    '00000000-0000-0000-0000-000000000902',
    id,
    position,
    sets,
    reps,
    rest_seconds
FROM (
    SELECT id, 2 AS position, 3 AS sets, 30 AS reps, 45 AS rest_seconds
    FROM exercises
    WHERE source = 'exercise-dataset' AND source_id = '0002'
    UNION ALL
    SELECT id, 1, 3, 12, 60
    FROM exercises
    WHERE source = 'exercise-dataset' AND source_id = '0001'
) AS dataset_exercises;

COMMIT;
SQL

public_response=$(mktemp)
private_response=$(mktemp)
exercises_response=$(mktemp)
trap 'rm -f "$public_response" "$private_response" "$exercises_response"' EXIT

public_status=$(curl -sS -o "$public_response" -w '%{http_code}' \
    "$base_url/api/v1/routines/demo")
private_status=$(curl -sS -o "$private_response" -w '%{http_code}' \
    "$base_url/api/v1/routines/$private_routine_id")
exercises_status=$(curl -sS -o "$exercises_response" -w '%{http_code}' \
    "$base_url/api/v1/exercises")

test "$public_status" = "200"
test "$private_status" = "404"
test "$exercises_status" = "200"

jq -e '
    (has("id") | not) and
    .name == "API Test Routine" and
    .description == "Routine returned by the API test." and
    (.author | has("id") | not) and
    .author.name == "API Test Author" and
    (.exercises | map(.position) == [1, 2]) and
    (.exercises | all(has("id") and (.id | test("^[0-9]{4}$")))) and
    .exercises[0].name == "3/4 sit-up" and
    (.exercises[0].gif_url | endswith("videos/0001-2gPfomN.gif")) and
    .exercises[1].name == "45° side bend"
' "$public_response" >/dev/null

jq -e '.error.code == "NOT_FOUND"' "$private_response" >/dev/null

jq -e '
    length >= 1324 and
    all(.[]; has("id") and (.id | test("^[0-9]{4}$"))) and
    any(.[]; .name == "3/4 sit-up" and (.gif_url | endswith("videos/0001-2gPfomN.gif"))) and
    (all(.[]; .name != "API Test Push Up" and .name != "API Test Plank"))
' "$exercises_response" >/dev/null

printf 'routine and exercise endpoints passed (routine=%s, private=%s, exercises=%s)\n' \
    "$public_status" "$private_status" "$exercises_status"
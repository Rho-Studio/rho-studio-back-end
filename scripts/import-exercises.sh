#!/usr/bin/env bash
set -euo pipefail

dataset_file="${1:-dataset/exercises.json}"

if [[ ! -f "$dataset_file" ]]; then
    printf 'dataset file not found: %s\n' "$dataset_file" >&2
    exit 1
fi

{
    printf '%s\n' \
        'CREATE TEMP TABLE exercise_import (payload jsonb);' \
        'COPY exercise_import (payload) FROM STDIN WITH (FORMAT csv);'
    jq -r '.[] | tojson | [.] | @csv' "$dataset_file"
    printf '%s\n' '\.'
    cat <<'SQL'
INSERT INTO exercises (
    name,
    slug,
    description,
    instructions,
    muscle_group,
    secondary_muscles,
    equipment,
    category,
    gif_storage_key,
    video_url,
    source,
    source_id,
    is_published
)
SELECT
    payload->>'name',
    'dataset-' || (payload->>'id'),
    payload->'instructions'->>'en',
    payload->'instructions'->>'en',
    payload->>'body_part',
    ARRAY(
        SELECT jsonb_array_elements_text(payload->'secondary_muscles')
    ),
    payload->>'equipment',
    payload->>'category',
    payload->>'gif_url',
    payload->>'gif_url',
    'exercise-dataset',
    payload->>'id',
    true
FROM exercise_import
ON CONFLICT (source, source_id) WHERE source IS NOT NULL AND source_id IS NOT NULL
DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    instructions = EXCLUDED.instructions,
    muscle_group = EXCLUDED.muscle_group,
    secondary_muscles = EXCLUDED.secondary_muscles,
    equipment = EXCLUDED.equipment,
    category = EXCLUDED.category,
    gif_storage_key = EXCLUDED.gif_storage_key,
    video_url = EXCLUDED.video_url,
    is_published = EXCLUDED.is_published,
    updated_at = now();

SELECT format('imported %s exercises', count(*)) FROM exercise_import;
SQL
} | docker compose exec -T postgres psql \
    -U rho_studio \
    -d rho_studio \
    -v ON_ERROR_STOP=1
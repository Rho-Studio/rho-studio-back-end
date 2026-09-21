#!/usr/bin/env bash
set -euo pipefail

base_url="${BASE_URL:-http://127.0.0.1:8080}"
response_file=$(mktemp)
trap 'rm -f "$response_file"' EXIT

if ! curl -fsS "$base_url/health" >/dev/null; then
    printf 'backend is not reachable at %s\n' "$base_url" >&2
    printf 'start local services with: docker compose up -d --build\n' >&2
    exit 1
fi

status=$(curl -sS -o "$response_file" -w '%{http_code}' \
    "$base_url/api/v1/exercises")
test "$status" = "200"

jq -e '
    length >= 1324 and
    all(.[]; has("id") and (.id | test("^[0-9]{4}$"))) and
    all(.[]; has("name") and has("description") and has("gif_url")) and
    any(.[]; .id == "0001" and .name == "3/4 sit-up")
' "$response_file" >/dev/null

printf 'exercise catalog endpoint passed (status=%s, count=%s)\n' \
    "$status" "$(jq length "$response_file")"
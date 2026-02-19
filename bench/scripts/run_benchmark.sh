#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/bench/docker-compose.yml"

cd "$ROOT_DIR"
docker compose -f "$COMPOSE_FILE" up -d --wait
trap 'docker compose -f "$COMPOSE_FILE" down -v >/dev/null 2>&1 || true' EXIT

export MYR_DB_PASSWORD="root"

cargo run -p myr-app --bin benchmark -- \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --seed-rows 50000 \
  --sql "SELECT id, user_id, category, payload, created_at FROM events ORDER BY id LIMIT 50000" \
  "$@"

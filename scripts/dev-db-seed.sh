#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/bench/docker-compose.yml"

DB_HOST="${MYR_DEV_DB_HOST:-127.0.0.1}"
DB_PORT="${MYR_DEV_DB_PORT:-33306}"
DB_USER="${MYR_DEV_DB_USER:-root}"
DB_PASSWORD="${MYR_DEV_DB_PASSWORD:-root}"
DB_NAME="${MYR_DEV_DB_NAME:-myr_bench}"
CONTAINER_NAME="${MYR_DEV_DB_CONTAINER:-myr-bench-mysql}"

cd "$ROOT_DIR"

docker compose -f "$COMPOSE_FILE" up -d --wait

docker exec -i "$CONTAINER_NAME" mysql -u"$DB_USER" -p"$DB_PASSWORD" "$DB_NAME" <<'SQL'
CREATE TABLE IF NOT EXISTS users (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  email VARCHAR(255) NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_users_email (email)
);

CREATE TABLE IF NOT EXISTS events (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  user_id BIGINT NOT NULL,
  category VARCHAR(32) NOT NULL,
  payload VARCHAR(128) NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_user_id_id (user_id, id),
  KEY idx_created_at (created_at)
);

INSERT INTO users (email) VALUES
  ('alice@example.com'),
  ('bob@example.com'),
  ('carol@example.com')
ON DUPLICATE KEY UPDATE
  email = VALUES(email);

TRUNCATE TABLE events;

INSERT INTO events (user_id, category, payload, created_at) VALUES
  (1, 'login', 'ok', NOW() - INTERVAL 30 MINUTE),
  (2, 'search', 'rust tui', NOW() - INTERVAL 28 MINUTE),
  (3, 'play', 'track-001', NOW() - INTERVAL 27 MINUTE),
  (1, 'pause', 'track-001', NOW() - INTERVAL 26 MINUTE),
  (2, 'skip', 'track-001', NOW() - INTERVAL 25 MINUTE),
  (3, 'share', 'playlist-alpha', NOW() - INTERVAL 24 MINUTE),
  (1, 'search', 'mysql async', NOW() - INTERVAL 23 MINUTE),
  (2, 'play', 'track-044', NOW() - INTERVAL 22 MINUTE),
  (3, 'pause', 'track-044', NOW() - INTERVAL 21 MINUTE),
  (1, 'skip', 'track-044', NOW() - INTERVAL 20 MINUTE),
  (2, 'share', 'playlist-beta', NOW() - INTERVAL 19 MINUTE),
  (3, 'login', 'ok', NOW() - INTERVAL 18 MINUTE),
  (1, 'search', 'pagination', NOW() - INTERVAL 17 MINUTE),
  (2, 'play', 'track-201', NOW() - INTERVAL 16 MINUTE),
  (3, 'pause', 'track-201', NOW() - INTERVAL 15 MINUTE),
  (1, 'skip', 'track-201', NOW() - INTERVAL 14 MINUTE),
  (2, 'share', 'playlist-gamma', NOW() - INTERVAL 13 MINUTE),
  (3, 'search', 'safe mode', NOW() - INTERVAL 12 MINUTE),
  (1, 'play', 'track-777', NOW() - INTERVAL 11 MINUTE),
  (2, 'login', 'ok', NOW() - INTERVAL 10 MINUTE);
SQL

docker exec -i "$CONTAINER_NAME" mysql -N -u"$DB_USER" -p"$DB_PASSWORD" "$DB_NAME" -e \
  "SELECT COUNT(*) AS users_count FROM users; SELECT COUNT(*) AS events_count FROM events;"

cat <<EOF
Dev MySQL dataset is ready.

Use these values in myr Connection Wizard:
  Host: $DB_HOST
  Port: $DB_PORT
  User: $DB_USER
  Database: $DB_NAME

Then set password in your shell:
  export MYR_DB_PASSWORD=$DB_PASSWORD

Smoke query:
  SELECT id, user_id, category, payload, created_at FROM events ORDER BY id LIMIT 20;
EOF

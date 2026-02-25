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
SET FOREIGN_KEY_CHECKS = 0;
DROP TABLE IF EXISTS events;
DROP TABLE IF EXISTS playlist_tracks;
DROP TABLE IF EXISTS playlists;
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS devices;
DROP TABLE IF EXISTS tracks;
DROP TABLE IF EXISTS artists;
DROP TABLE IF EXISTS users;
DROP TABLE IF EXISTS organizations;
SET FOREIGN_KEY_CHECKS = 1;

CREATE TABLE organizations (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  slug VARCHAR(64) NOT NULL,
  name VARCHAR(128) NOT NULL,
  plan_tier ENUM('free', 'pro', 'enterprise') NOT NULL DEFAULT 'free',
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_org_slug (slug)
);

CREATE TABLE users (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  organization_id BIGINT NOT NULL,
  email VARCHAR(255) NOT NULL,
  display_name VARCHAR(80) NOT NULL,
  status ENUM('active', 'paused', 'invited') NOT NULL DEFAULT 'active',
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_login_at DATETIME NULL,
  UNIQUE KEY uq_users_email (email),
  KEY idx_users_org_status (organization_id, status),
  CONSTRAINT fk_users_organization
    FOREIGN KEY (organization_id) REFERENCES organizations(id)
);

CREATE TABLE devices (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  user_id BIGINT NOT NULL,
  platform ENUM('ios', 'android', 'web', 'desktop') NOT NULL,
  app_version VARCHAR(16) NOT NULL,
  last_seen_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_devices_user_seen (user_id, last_seen_at),
  CONSTRAINT fk_devices_user
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE TABLE sessions (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  user_id BIGINT NOT NULL,
  device_id BIGINT NOT NULL,
  started_at DATETIME NOT NULL,
  ended_at DATETIME NULL,
  ip_address VARCHAR(45) NOT NULL,
  country_code CHAR(2) NOT NULL,
  KEY idx_sessions_user_started (user_id, started_at),
  KEY idx_sessions_country_started (country_code, started_at),
  CONSTRAINT fk_sessions_user
    FOREIGN KEY (user_id) REFERENCES users(id),
  CONSTRAINT fk_sessions_device
    FOREIGN KEY (device_id) REFERENCES devices(id)
);

CREATE TABLE artists (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(120) NOT NULL,
  country_code CHAR(2) NOT NULL,
  debut_year SMALLINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_artists_name (name)
);

CREATE TABLE tracks (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  artist_id BIGINT NOT NULL,
  title VARCHAR(120) NOT NULL,
  genre VARCHAR(32) NOT NULL,
  duration_sec INT NOT NULL,
  release_date DATE NULL,
  is_explicit TINYINT(1) NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_tracks_artist_title (artist_id, title),
  KEY idx_tracks_genre_release (genre, release_date),
  CONSTRAINT fk_tracks_artist
    FOREIGN KEY (artist_id) REFERENCES artists(id)
);

CREATE TABLE playlists (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  user_id BIGINT NOT NULL,
  name VARCHAR(120) NOT NULL,
  mood VARCHAR(32) NOT NULL,
  is_public TINYINT(1) NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  KEY idx_playlists_user_updated (user_id, updated_at),
  CONSTRAINT fk_playlists_user
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE TABLE playlist_tracks (
  playlist_id BIGINT NOT NULL,
  track_id BIGINT NOT NULL,
  position INT NOT NULL,
  added_by_user_id BIGINT NOT NULL,
  added_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (playlist_id, track_id),
  UNIQUE KEY uq_playlist_position (playlist_id, position),
  CONSTRAINT fk_playlist_tracks_playlist
    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
  CONSTRAINT fk_playlist_tracks_track
    FOREIGN KEY (track_id) REFERENCES tracks(id),
  CONSTRAINT fk_playlist_tracks_added_by
    FOREIGN KEY (added_by_user_id) REFERENCES users(id)
);

CREATE TABLE events (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  user_id BIGINT NOT NULL,
  session_id BIGINT NULL,
  track_id BIGINT NULL,
  playlist_id BIGINT NULL,
  device_id BIGINT NULL,
  category VARCHAR(32) NOT NULL,
  payload VARCHAR(128) NOT NULL,
  metadata JSON NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_user_id_id (user_id, id),
  KEY idx_category_created (category, created_at),
  KEY idx_created_at (created_at),
  CONSTRAINT fk_events_user
    FOREIGN KEY (user_id) REFERENCES users(id),
  CONSTRAINT fk_events_session
    FOREIGN KEY (session_id) REFERENCES sessions(id),
  CONSTRAINT fk_events_track
    FOREIGN KEY (track_id) REFERENCES tracks(id),
  CONSTRAINT fk_events_playlist
    FOREIGN KEY (playlist_id) REFERENCES playlists(id),
  CONSTRAINT fk_events_device
    FOREIGN KEY (device_id) REFERENCES devices(id)
);

INSERT INTO organizations (slug, name, plan_tier, created_at) VALUES
  ('acme', 'Acme Audio', 'enterprise', NOW() - INTERVAL 90 DAY),
  ('lowfi-labs', 'Lowfi Labs', 'pro', NOW() - INTERVAL 60 DAY),
  ('sunset-data', 'Sunset Data', 'free', NOW() - INTERVAL 45 DAY);

INSERT INTO users (organization_id, email, display_name, status, created_at, last_login_at) VALUES
  (1, 'alice@example.com', 'Alice', 'active', NOW() - INTERVAL 50 DAY, NOW() - INTERVAL 15 MINUTE),
  (1, 'bob@example.com', 'Bob', 'active', NOW() - INTERVAL 48 DAY, NOW() - INTERVAL 10 MINUTE),
  (2, 'carol@example.com', 'Carol', 'paused', NOW() - INTERVAL 40 DAY, NOW() - INTERVAL 3 HOUR),
  (2, 'diego@example.com', 'Diego', 'active', NOW() - INTERVAL 35 DAY, NOW() - INTERVAL 20 MINUTE),
  (3, 'eve@example.com', 'Eve', 'invited', NOW() - INTERVAL 20 DAY, NULL),
  (3, 'fran@example.com', 'Fran', 'active', NOW() - INTERVAL 18 DAY, NOW() - INTERVAL 40 MINUTE),
  (1, 'gio@example.com', 'Gio', 'active', NOW() - INTERVAL 10 DAY, NOW() - INTERVAL 5 MINUTE),
  (2, 'hana@example.com', 'Hana', 'active', NOW() - INTERVAL 8 DAY, NOW() - INTERVAL 25 MINUTE);

INSERT INTO devices (user_id, platform, app_version, last_seen_at, created_at) VALUES
  (1, 'web', '2.1.0', NOW() - INTERVAL 10 MINUTE, NOW() - INTERVAL 180 DAY),
  (1, 'ios', '2.0.1', NOW() - INTERVAL 25 MINUTE, NOW() - INTERVAL 90 DAY),
  (2, 'android', '2.1.0', NOW() - INTERVAL 7 MINUTE, NOW() - INTERVAL 110 DAY),
  (3, 'desktop', '1.9.4', NOW() - INTERVAL 4 HOUR, NOW() - INTERVAL 130 DAY),
  (4, 'web', '2.1.1', NOW() - INTERVAL 12 MINUTE, NOW() - INTERVAL 80 DAY),
  (5, 'ios', '2.1.1', NULL, NOW() - INTERVAL 20 DAY),
  (6, 'android', '2.0.9', NOW() - INTERVAL 1 HOUR, NOW() - INTERVAL 70 DAY),
  (7, 'desktop', '2.2.0', NOW() - INTERVAL 5 MINUTE, NOW() - INTERVAL 15 DAY),
  (8, 'web', '2.1.1', NOW() - INTERVAL 15 MINUTE, NOW() - INTERVAL 12 DAY),
  (8, 'android', '2.1.0', NOW() - INTERVAL 30 MINUTE, NOW() - INTERVAL 12 DAY);

INSERT INTO sessions (user_id, device_id, started_at, ended_at, ip_address, country_code) VALUES
  (1, 1, NOW() - INTERVAL 5 HOUR, NOW() - INTERVAL 4 HOUR, '192.168.10.10', 'US'),
  (1, 2, NOW() - INTERVAL 90 MINUTE, NOW() - INTERVAL 60 MINUTE, '10.0.0.2', 'US'),
  (2, 3, NOW() - INTERVAL 70 MINUTE, NOW() - INTERVAL 30 MINUTE, '10.0.0.3', 'US'),
  (3, 4, NOW() - INTERVAL 8 HOUR, NOW() - INTERVAL 7 HOUR, '10.0.0.4', 'AR'),
  (4, 5, NOW() - INTERVAL 45 MINUTE, NULL, '10.0.0.5', 'MX'),
  (5, 6, NOW() - INTERVAL 1 DAY, NOW() - INTERVAL 23 HOUR, '10.0.0.6', 'BR'),
  (6, 7, NOW() - INTERVAL 2 HOUR, NOW() - INTERVAL 90 MINUTE, '10.0.0.7', 'CL'),
  (7, 8, NOW() - INTERVAL 35 MINUTE, NULL, '10.0.0.8', 'US'),
  (8, 9, NOW() - INTERVAL 55 MINUTE, NOW() - INTERVAL 15 MINUTE, '10.0.0.9', 'UY'),
  (8, 10, NOW() - INTERVAL 20 MINUTE, NULL, '10.0.0.10', 'UY'),
  (2, 3, NOW() - INTERVAL 3 DAY, NOW() - INTERVAL 3 DAY + INTERVAL 50 MINUTE, '10.0.1.3', 'US'),
  (4, 5, NOW() - INTERVAL 2 DAY, NOW() - INTERVAL 2 DAY + INTERVAL 1 HOUR, '10.0.1.5', 'MX');

INSERT INTO artists (name, country_code, debut_year, created_at) VALUES
  ('The Violet Arrays', 'US', 2011, NOW() - INTERVAL 4 YEAR),
  ('Neon Rivers', 'AR', 2014, NOW() - INTERVAL 3 YEAR),
  ('Paper Satellites', 'BR', 2018, NOW() - INTERVAL 2 YEAR),
  ('Atlas Bloom', 'CL', 2020, NOW() - INTERVAL 1 YEAR),
  ('Night Transit', 'US', 2008, NOW() - INTERVAL 6 YEAR);

INSERT INTO tracks (artist_id, title, genre, duration_sec, release_date, is_explicit, created_at) VALUES
  (1, 'Open Fields', 'indie', 214, '2021-03-12', 0, NOW() - INTERVAL 400 DAY),
  (1, 'Broken Compass', 'indie', 198, '2022-06-21', 0, NOW() - INTERVAL 320 DAY),
  (2, 'Northern Lights', 'electronic', 245, '2020-11-01', 0, NOW() - INTERVAL 500 DAY),
  (2, 'Friction', 'electronic', 189, '2023-01-11', 1, NOW() - INTERVAL 200 DAY),
  (3, 'Crane Song', 'ambient', 300, '2022-09-30', 0, NOW() - INTERVAL 250 DAY),
  (3, 'Wind Tunnel', 'ambient', 278, '2023-08-05', 0, NOW() - INTERVAL 150 DAY),
  (4, 'Volcano Child', 'alternative', 232, '2024-04-19', 1, NOW() - INTERVAL 80 DAY),
  (4, 'Nocturne Grid', 'alternative', 206, '2024-10-02', 0, NOW() - INTERVAL 40 DAY),
  (5, 'Pastel Engine', 'synthwave', 261, '2019-02-14', 0, NOW() - INTERVAL 700 DAY),
  (5, 'City Relay', 'synthwave', 247, '2021-12-08', 0, NOW() - INTERVAL 380 DAY);

INSERT INTO playlists (user_id, name, mood, is_public, created_at, updated_at) VALUES
  (1, 'Morning Focus', 'focus', 1, NOW() - INTERVAL 30 DAY, NOW() - INTERVAL 2 HOUR),
  (2, 'Night Drive', 'energy', 1, NOW() - INTERVAL 20 DAY, NOW() - INTERVAL 3 HOUR),
  (4, 'Bugfix Loops', 'deep-work', 0, NOW() - INTERVAL 12 DAY, NOW() - INTERVAL 40 MINUTE),
  (6, 'Soft Rain', 'calm', 1, NOW() - INTERVAL 5 DAY, NOW() - INTERVAL 90 MINUTE),
  (7, 'Weekly Picks', 'mixed', 0, NOW() - INTERVAL 3 DAY, NOW() - INTERVAL 30 MINUTE);

INSERT INTO playlist_tracks (playlist_id, track_id, position, added_by_user_id, added_at) VALUES
  (1, 1, 1, 1, NOW() - INTERVAL 28 DAY),
  (1, 5, 2, 1, NOW() - INTERVAL 28 DAY),
  (1, 8, 3, 1, NOW() - INTERVAL 27 DAY),
  (2, 3, 1, 2, NOW() - INTERVAL 19 DAY),
  (2, 4, 2, 2, NOW() - INTERVAL 19 DAY),
  (2, 9, 3, 2, NOW() - INTERVAL 18 DAY),
  (3, 2, 1, 4, NOW() - INTERVAL 10 DAY),
  (3, 6, 2, 4, NOW() - INTERVAL 10 DAY),
  (3, 7, 3, 4, NOW() - INTERVAL 10 DAY),
  (4, 5, 1, 6, NOW() - INTERVAL 4 DAY),
  (4, 6, 2, 6, NOW() - INTERVAL 4 DAY),
  (4, 10, 3, 6, NOW() - INTERVAL 4 DAY),
  (5, 1, 1, 7, NOW() - INTERVAL 2 DAY),
  (5, 4, 2, 7, NOW() - INTERVAL 2 DAY),
  (5, 8, 3, 7, NOW() - INTERVAL 2 DAY);

INSERT INTO events (
  user_id,
  session_id,
  track_id,
  playlist_id,
  device_id,
  category,
  payload,
  metadata,
  created_at
)
WITH RECURSIVE seq AS (
  SELECT 1 AS n
  UNION ALL
  SELECT n + 1 FROM seq WHERE n < 240
),
events_seed AS (
  SELECT
    n,
    ((n - 1) % 8) + 1 AS user_id,
    ((n - 1) % 12) + 1 AS session_id,
    ((n - 1) % 10) + 1 AS track_id,
    ((n - 1) % 5) + 1 AS playlist_id,
    ((n - 1) % 10) + 1 AS device_id,
    ELT(((n - 1) % 8) + 1,
      'login',
      'search',
      'play',
      'pause',
      'skip',
      'share',
      'like',
      'add_to_playlist'
    ) AS category
  FROM seq
)
SELECT
  user_id,
  session_id,
  track_id,
  playlist_id,
  device_id,
  category,
  CASE category
    WHEN 'login' THEN 'ok'
    WHEN 'search' THEN CONCAT('query-', LPAD(n, 3, '0'))
    WHEN 'play' THEN CONCAT('track-', LPAD(track_id, 3, '0'))
    WHEN 'pause' THEN CONCAT('track-', LPAD(track_id, 3, '0'))
    WHEN 'skip' THEN CONCAT('track-', LPAD(track_id, 3, '0'))
    WHEN 'share' THEN CONCAT('playlist-', LPAD(playlist_id, 2, '0'))
    WHEN 'like' THEN CONCAT('track-', LPAD(track_id, 3, '0'))
    ELSE CONCAT('playlist-', LPAD(playlist_id, 2, '0'))
  END AS payload,
  JSON_OBJECT(
    'source', ELT(((n - 1) % 4) + 1, 'home', 'search', 'recommendation', 'playlist'),
    'latency_ms', 40 + (n % 180),
    'ab_bucket', ELT(((n - 1) % 3) + 1, 'control', 'variant_a', 'variant_b'),
    'app_version', ELT(((n - 1) % 5) + 1, '2.0.9', '2.1.0', '2.1.1', '2.2.0', '2.2.1')
  ) AS metadata,
  NOW() - INTERVAL (300 - n) MINUTE AS created_at
FROM events_seed;
SQL

docker exec -i "$CONTAINER_NAME" mysql -N -u"$DB_USER" -p"$DB_PASSWORD" "$DB_NAME" -e \
  "SELECT COUNT(*) AS organizations_count FROM organizations;
   SELECT COUNT(*) AS users_count FROM users;
   SELECT COUNT(*) AS sessions_count FROM sessions;
   SELECT COUNT(*) AS tracks_count FROM tracks;
   SELECT COUNT(*) AS playlists_count FROM playlists;
   SELECT COUNT(*) AS events_count FROM events;"

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

Relationship smoke query:
  SELECT e.id, u.email, p.name AS playlist, t.title AS track, e.category, e.payload
  FROM events e
  JOIN users u ON u.id = e.user_id
  LEFT JOIN playlists p ON p.id = e.playlist_id
  LEFT JOIN tracks t ON t.id = e.track_id
  ORDER BY e.id DESC
  LIMIT 15;
EOF

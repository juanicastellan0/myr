# Manual Testing

Manual smoke plan for validating TUI behavior and MySQL connectivity.

## Prerequisites

1. Seed local dataset:

```bash
scripts/dev-db-seed.sh
```

2. Export password used by the TUI:

```bash
export MYR_DB_PASSWORD=root
```

3. Start app:

```bash
cargo run -p myr-app
```

## Connection Wizard Path

Use these seeded values:

- Host: `127.0.0.1`
- Port: `33306`
- User: `root`
- Password source (env/keyring): `env`
- Database: `myr_bench`
- TLS mode (disabled/prefer/require/verify_identity): `prefer`
- Read-only (yes/no): `no`

Steps:

1. Open Connection Wizard (`F6` if needed).
2. Edit fields (`E`/`Enter`) and save with `Enter`.
3. Connect with `F5`.

Expected:

- Runtime bar transitions `DB: [~] CONNECTING` to `DB: [+] CONNECTED`.
- Runtime bar shows `Mode: RW` for this profile.
- Runtime bar shows `TLS: prefer` for this profile.
- Status line reports successful connect with latency.
- App switches to Schema Explorer automatically.

## Secure Password Storage (Keyring)

Steps:

1. In Connection Wizard, set `Password source (env/keyring)` to `keyring`.
2. Ensure `MYR_DB_PASSWORD` is exported, then connect once with `F5`.
3. Exit app, unset the env var (`unset MYR_DB_PASSWORD`), and start app again.
4. Reconnect using the same profile.

Expected:

- First connect succeeds and keyring storage is attempted for that profile.
- Second connect works without `MYR_DB_PASSWORD` when OS keyring access is available.
- If keyring is unavailable, connect should fail with a clear auth/connect error.

## TLS Modes and Profile TLS Options

Steps:

1. In Connection Wizard, set `TLS mode` to `require` or `verify_identity`.
2. Connect to a TLS-enabled MySQL instance.
3. Optional: edit profile file (`~/.config/myr/profiles.toml` or `$MYR_CONFIG_DIR/myr/profiles.toml`) and set:
   - `tls_ca_cert_path`
   - `tls_client_cert_path`
   - `tls_client_key_path`
   - `tls_skip_domain_validation` / `tls_accept_invalid_certs` only for test/non-prod
4. Reconnect and rerun query smoke tests.

Expected:

- Runtime bar reflects selected TLS mode.
- TLS profile options are accepted and used by the adapter.
- Strict verification should reject invalid/untrusted cert chains.

## Profile Config Migration

Steps:

1. Exit the app.
2. Edit `~/.config/myr/profiles.toml` (or `$MYR_CONFIG_DIR/myr/profiles.toml`) to a legacy shape:
   - use `[[connections]]` instead of `[[profiles]]`
   - use legacy keys like `quick_connect`, `password_provider`, and `tls_ca_cert`
3. Start the app (`cargo run -p myr-app`) and open Connection Wizard/manager once.
4. Exit and reopen the profile file.

Expected:

- Legacy profile still loads in the app.
- File is rewritten to canonical format with `version = 1`.
- Legacy keys are replaced with canonical keys (`quick_reconnect`, `password_source`, `tls_ca_cert_path`).

## Pane Navigation and Animation

Steps:

1. Press `Tab` to cycle panes.
2. Press `F6` to jump back to Connection Wizard.

Expected:

- Active pane changes in top tabs.
- Active tab briefly flashes on pane change.
- Status line shows pane switch messages.

## Schema Explorer Filter-as-you-Type

Steps:

1. Go to Schema Explorer.
2. Keep focus on `Tables` lane and type `sess`.
3. Confirm selection moves to `sessions`.
4. Press `Ctrl+U` to clear the lane filter.
5. Move to `Columns` lane (`Right`) and type `upd`.
6. Confirm selection moves to `updated_at`.
7. Press `Backspace` and then `Ctrl+U`.
8. Press `F4` to switch to full metadata mode; press `F4` again to return to compact mode.

Expected:

- Typed text filters only the active lane (database/table/column).
- Selection is constrained to matching entries and `Up`/`Down` navigate within matches.
- Section headers show match counts and the active filter text.
- `Backspace` removes one filter character and `Ctrl+U` clears the active lane filter.
- Columns lane can switch between compact names and full metadata display (`name | type | nullability | default`).

## Query and Results

Steps:

1. Go to Query Editor.
2. Run:

```sql
SELECT id, user_id, category, payload, created_at
FROM `myr_bench`.`events`
ORDER BY id
LIMIT 20;
```

3. Press `Enter`.

Expected:

- Runtime bar shows query activity while running.
- Results pane shows rows.
- Status line reports `Query returned ... rows`.

### Complex Join + JSON Projection

Steps:

1. In Query Editor, run:

```sql
SELECT
  e.id,
  u.email,
  t.title AS track,
  p.name AS playlist,
  e.category,
  JSON_UNQUOTE(JSON_EXTRACT(e.metadata, '$.source')) AS source
FROM `myr_bench`.`events` e
JOIN `myr_bench`.`users` u ON u.id = e.user_id
LEFT JOIN `myr_bench`.`tracks` t ON t.id = e.track_id
LEFT JOIN `myr_bench`.`playlists` p ON p.id = e.playlist_id
ORDER BY e.id DESC
LIMIT 25;
```

Expected:

- Query succeeds with mixed `JOIN` + `LEFT JOIN` paths.
- Results render both scalar columns and JSON-derived values (`source`).
- Schema Explorer relationship lanes remain responsive after this query.

### Results Table Horizontal Viewport

Steps:

1. In Results pane, press `Right` several times to move active column.
2. Press `Left` to move back.
3. Observe the table header and viewport metadata line.

Expected:

- `Left/Right` changes the active result column without changing selected row.
- Header/cell rendering visually emphasizes the active column.
- When terminal width is narrow, viewport line reports visible column range (for example `Columns 3-5 / 8`).

## Read-only Profile Guard

Steps:

1. Go to Connection Wizard (`F6`).
2. Set `Read-only (yes/no)` to `yes`.
3. Connect with `F5`.
4. In Query Editor, run a write statement such as:

```sql
DELETE FROM `myr_bench`.`events` WHERE id = 1;
```

Expected:

- Runtime bar shows `Mode: RO`.
- Query is blocked before execution.
- Status line reports `Blocked by read-only profile mode: write/DDL SQL is disabled`.

## Query Editor Usability

### Multiline, Cursor, and History

Steps:

1. Go to Query Editor and clear it (`Ctrl+U`).
2. Type `SELECT id` then press `Ctrl+Enter` (or `Ctrl+J`) to insert a new line.
3. Type `FROM \`myr_bench\`.\`events\`` and press `Ctrl+Enter` again.
4. Type `LIMIT 5;` and use `Left`/`Right` to move the cursor.
5. Press `Enter` to run the query.
6. Return to Query Editor and use `Up`/`Down` to cycle query history.

Expected:

- Query editor renders multiple numbered lines.
- Query editor shows an explicit column ruler and a bounded SQL region (`SQL (active region)` ... `End SQL`).
- Cursor movement updates line/column info in the editor footer.
- For long multiline SQL, editor shows a viewport with `... lines above/below` indicators while keeping cursor line visible.
- History navigation restores previously executed SQL and can return to the draft query.

### Snippet Insert Actions

Steps:

1. In Query Editor, open command palette (`Ctrl+P`).
2. Search `snippet` and invoke `Insert SELECT snippet`.
3. Open command palette again and invoke `Insert JOIN snippet`.

Expected:

- Snippets are inserted into the editor at the current cursor position.
- App keeps focus on Query Editor and status line reports snippet insertion.

### Command Palette Fuzzy/Alias Search

Steps:

1. Open command palette (`Ctrl+P`) from Schema Explorer.
2. Type `pvw` and confirm `Preview table` is shown as a top match.
3. Clear query and type `ddl`; confirm `Show create table` is shown.
4. Clear query and type `fk`; confirm `Jump to related table` is shown.

Expected:

- Palette matches actions even when query is not a direct substring.
- Action aliases/keywords map to intended actions (`pvw`, `ddl`, `fk`).

## Guided Query Actions

### Server-side Filter/Sort Builder

Steps:

1. Go to Schema Explorer and highlight a concrete database/table/column.
2. Open command palette (`Ctrl+P`) and invoke `Build filter/sort query`.

Expected:

- Query Editor is populated with a generated query:
  - `WHERE <column> LIKE '%search%'`
  - `ORDER BY <column> ASC`
  - `LIMIT 200`
- App switches to Query Editor and reports `Query editor updated`.

### EXPLAIN Preflight Action

Steps:

1. Put a normal `SELECT ...` query in Query Editor.
2. Open command palette and invoke `Explain query`.

Expected:

- Action runs `EXPLAIN <query>` instead of replacing editor text.
- Results pane displays the MySQL execution plan rows.

## Foreign-key Relationship Jump

Steps:

1. Go to Schema Explorer and select a table with known relationships (for seeded data, `users`, `sessions`, or `events`).
2. Confirm the `Relationships` subsection shows related table entries.
3. Invoke `Jump to related table` from command palette (`Ctrl+P`) or its ranked footer slot.
4. Invoke the same action again to continue cycling relationships.

Expected:

- Selection jumps to the related database/table/column target.
- Query editor text updates to the newly selected table.
- Status line reports the relationship constraint used for the jump.

## Saved Bookmarks

Steps:

1. Select a schema target and place a query in Query Editor.
2. Invoke `Save bookmark` from command palette.
3. Change selection/query to something else.
4. Invoke `Open bookmark` to load a saved entry.
5. Repeat `Open bookmark` to cycle additional saved entries.

Expected:

- Bookmark save reports a generated bookmark name and total count.
- Open restores database/table/column selection and query text.
- Bookmarks persist to `~/.config/myr/bookmarks.toml` (or `$MYR_CONFIG_DIR/myr/bookmarks.toml`).

## Profiles and Bookmarks Manager

Steps:

1. Press `F7` to open the `Profiles & Bookmarks` pane.
2. In `Profiles` lane, use `Up`/`Down` and press `Enter` on a profile.
3. Confirm app returns to Connection Wizard with profile fields loaded.
4. Press `F7`, select a profile, press `r`, type a new name, then press `Enter` to save rename.
5. In `Profiles` lane, press `d` on a profile to mark default.
6. In `Profiles` lane, press `q` on a profile to mark quick reconnect target.
7. Move to `Bookmarks` lane (`Right`) and press `F5`.
8. Select a bookmark and press `Enter` to open it.
9. Press `Del` on a selected bookmark entry.

Expected:

- Manager pane shows both profile and bookmark lists with focused lane indicator.
- Opening a profile loads it into Connection Wizard for quick reconnect (`F5`).
- Rename mode shows inline input with `Enter` save and `Esc` cancel behavior.
- Profile list row shows `[default]` and/or `[quick]` marker tags when set.
- Pressing `F5` from manager connects using selected profile (Profiles lane) or quick reconnect target (Bookmarks lane fallback).
- Opening a bookmark restores its selection/query target.
- `Del` removes selected entry and persists updated store file.

## Results Search Mode

Steps:

1. Run any query that returns rows.
2. Trigger search action (`Ctrl+P` -> `Search results` or action slot key shown in footer).
3. Enter a search term and press `Enter`.
4. Press `Enter` to jump to the next match (or `Esc` to exit search mode).

Expected:

- Status line shows match count and active match index.
- Result cursor jumps between matched rows.

## Extended Export Formats

Steps:

1. Run any query that returns rows.
2. Invoke each export action from command palette:
   - `Export CSV`
   - `Export JSON`
   - `Export CSV (gzip)`
   - `Export JSON (gzip)`
   - `Export JSONL`
   - `Export JSONL (gzip)`
3. For gzip outputs, validate file contents with:

```bash
gzip -dc /tmp/myr-export-*.csv.gz | head
gzip -dc /tmp/myr-export-*.jsonl.gz | head
```

Expected:

- Each action reports successful export with output path.
- Gzip files decompress correctly.
- JSONL exports contain one JSON object per line.

## SQL Audit Trail

Steps:

1. Run one successful query and one failing query from Query Editor.
2. Inspect audit file:

```bash
tail -n 20 ~/.config/myr/audit.ndjson
```

or if using custom config root:

```bash
tail -n 20 "$MYR_CONFIG_DIR/myr/audit.ndjson"
```

3. Trigger aggressive rotation and run a few more queries:

```bash
export MYR_AUDIT_MAX_BYTES=512
export MYR_AUDIT_MAX_ARCHIVES=2
```

4. Restart `myr-app`, run several queries, then inspect rotated files:

```bash
ls -1 ~/.config/myr/audit.ndjson*
```

Expected:

- Each query lifecycle emits JSON-line records with:
  - `timestamp_unix_ms`
  - `profile_name`
  - `database`
  - `outcome` (`started`, `succeeded`, `failed`, `cancelled`, `blocked`)
- Success records include row/elapsed metadata; failed/blocked records include `error`.
- When size threshold is exceeded, audit file rotates to `audit.ndjson.1`, then `audit.ndjson.2`, keeping at most configured archive count.

## Resilience and Recovery

### Health Diagnostics Action

Steps:

1. Open command palette (`Ctrl+P`), search `health`, and run `Run health diagnostics`.
2. Confirm behavior while connected.
3. Disconnect DB (or use invalid connection), rerun diagnostics, and inspect error panel.

Expected:

- Connected run reports: connection check OK, schema check OK, query smoke OK.
- Failure run opens `Health Diagnostics` panel with failing check details.

### Auto-Reconnect State Flow

Steps:

1. Start a query in Query Editor.
2. While query runs, stop the DB container temporarily:

```bash
docker stop myr-bench-mysql
```

3. Watch the runtime bar and status line.

Expected:

- Runtime DB state transitions to `RECONNECTING`.
- Status line indicates reconnect attempts.
- If reconnect succeeds after DB is back, query is retried automatically.

### Error Panel and Guided Recovery

Steps:

1. Trigger a connection/query failure (for example by using a wrong port or stopping DB).
2. Observe the error panel popup.
3. Use recovery shortcuts:
   - `1` or `Enter`: run primary action
   - `F5`: reconnect now
   - `F6`: open Connection Wizard
   - `Esc`: dismiss panel

Expected:

- Error panel includes failure detail and recovery options.
- Recovery shortcuts perform the mapped actions and update status line.

## Exit Paths

Steps and expected behavior:

1. Press `Ctrl+C` while idle:
   - Confirm-exit modal appears.
2. Press `Esc`:
   - Exit is canceled and app returns.
3. Press `Ctrl+C` again from idle confirm state:
   - App exits.
4. Press `F10` from any pane:
   - Immediate exit.

## Non-Interactive CLI Entry Points

Prerequisite: keep `MYR_DB_PASSWORD` exported and local seed DB running (`scripts/dev-db-seed.sh`).

### Query (`myr-app query`)

Steps:

1. Run:

```bash
MYR_DB_PASSWORD=root \
cargo run -p myr-app -- \
  query \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --sql "SELECT id, email FROM \`myr_bench\`.\`users\` ORDER BY id LIMIT 2"
```

Expected:

- `stdout` prints one JSON object per row.
- Process exits with code `0`.

### Export (`myr-app export`)

Steps:

1. Run:

```bash
MYR_DB_PASSWORD=root \
cargo run -p myr-app -- \
  export \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --sql "SELECT id, category, payload FROM \`myr_bench\`.\`events\` ORDER BY id LIMIT 50" \
  --format csv.gz \
  --output /tmp/myr-cli-export.csv.gz
```

2. Verify generated file:

```bash
gzip -dc /tmp/myr-cli-export.csv.gz | head -n 3
```

Expected:

- Command reports `export.rows_written=50` (or matching selected row count).
- Decompressed output includes a CSV header row and query data rows.

### Doctor (`myr-app doctor`)

Steps:

1. Run:

```bash
MYR_DB_PASSWORD=root \
cargo run -p myr-app -- \
  doctor \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench
```

Expected:

- Output reports `doctor.connection=ok`, `doctor.schema=ok`, `doctor.query_smoke=ok`.
- Command exits with code `0`.

## Optional Real-DB Automated Checks

Adapter integration:

```bash
MYR_DB_PASSWORD=root \
MYR_RUN_MYSQL_INTEGRATION=1 \
cargo test -p myr-adapters --test mysql_integration -- --nocapture
```

Adapter integration (MariaDB target, optional):

```bash
# Example MariaDB service:
# docker run --rm --name myr-mariadb -e MARIADB_ROOT_PASSWORD=root -p 33307:3306 mariadb:11.4
MYR_DB_PASSWORD=root \
MYR_RUN_MYSQL_INTEGRATION=1 \
MYR_TEST_DB_HOST=127.0.0.1 \
MYR_TEST_DB_PORT=33307 \
MYR_TEST_DB_USER=root \
cargo test -p myr-adapters --test mysql_integration -- --nocapture
```

TUI MySQL query-path integration:

```bash
MYR_DB_PASSWORD=root \
MYR_RUN_TUI_MYSQL_INTEGRATION=1 \
MYR_TEST_DB_HOST=127.0.0.1 \
MYR_TEST_DB_PORT=33306 \
MYR_TEST_DB_USER=root \
MYR_TEST_DB_DATABASE=myr_bench \
cargo test -p myr-tui mysql_query_path_streams_rows_when_enabled -- --nocapture
```

Keyring smoke check (optional; Linux/macOS/Windows):

```bash
MYR_RUN_KEYRING_SMOKE=1 \
cargo test -p myr-adapters keyring_password_round_trip_when_enabled -- --nocapture
```

Perf metrics output (for trend artifacts / local tracking):

```bash
MYR_DB_PASSWORD=root \
cargo run -p myr-app --bin benchmark -- \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --seed-rows 10000 \
  --metrics-label local-smoke \
  --metrics-output target/perf/local-smoke.json
```

Perf trend guard check (baseline + tolerance windows):

```bash
MYR_DB_PASSWORD=root \
cargo run -p myr-app --bin benchmark -- \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --seed-rows 10000 \
  --sql "SELECT id, user_id, category, payload, created_at FROM events ORDER BY id LIMIT 10000" \
  --trend-policy bench/perf-trend-policy.json
```

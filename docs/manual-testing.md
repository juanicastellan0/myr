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

## Pane Navigation and Animation

Steps:

1. Press `Tab` to cycle panes.
2. Press `F6` to jump back to Connection Wizard.

Expected:

- Active pane changes in top tabs.
- Active tab briefly flashes on pane change.
- Status line shows pane switch messages.

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
- Cursor movement updates line/column info in the editor footer.
- History navigation restores previously executed SQL and can return to the draft query.

### Snippet Insert Actions

Steps:

1. In Query Editor, open command palette (`Ctrl+P`).
2. Search `snippet` and invoke `Insert SELECT snippet`.
3. Open command palette again and invoke `Insert JOIN snippet`.

Expected:

- Snippets are inserted into the editor at the current cursor position.
- App keeps focus on Query Editor and status line reports snippet insertion.

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

1. Go to Schema Explorer and select a table with known relationships (for seeded data, `users`/`events`).
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

Expected:

- Each query lifecycle emits JSON-line records with:
  - `timestamp_unix_ms`
  - `profile_name`
  - `database`
  - `outcome` (`started`, `succeeded`, `failed`, `cancelled`, `blocked`)
- Success records include row/elapsed metadata; failed/blocked records include `error`.

## Resilience and Recovery

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

## Optional Real-DB Automated Checks

Adapter integration:

```bash
MYR_DB_PASSWORD=root \
MYR_RUN_MYSQL_INTEGRATION=1 \
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

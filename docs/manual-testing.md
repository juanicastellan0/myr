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
- Database: `myr_bench`

Steps:

1. Open Connection Wizard (`F6` if needed).
2. Edit fields (`E`/`Enter`) and save with `Enter`.
3. Connect with `F5`.

Expected:

- Runtime bar transitions `DB: [~] CONNECTING` to `DB: [+] CONNECTED`.
- Status line reports successful connect with latency.
- App switches to Schema Explorer automatically.

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

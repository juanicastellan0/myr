# Quality Gates

This project enforces quality with local checks and CI gates.

## Local Commands

Run from repository root:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

## Coverage

Generate a local HTML report:

```bash
cargo llvm-cov --workspace --all-features --html --output-dir target/coverage/html
```

Print a table summary:

```bash
cargo llvm-cov report --summary-only
```

## CI Workflow

See `.github/workflows/ci.yml` for gate configuration.

Current gate settings:
- line coverage threshold: `85%`
- MySQL integration coverage enabled with:
  - `MYR_DB_PASSWORD=root`
  - `MYR_RUN_MYSQL_INTEGRATION=1`
- MariaDB compatibility lane:
  - service image: `mariadb:11.4`
  - command: `cargo test -p myr-adapters --test mysql_integration -- --nocapture`
- gated TUI MySQL query-path integration:
  - `MYR_DB_PASSWORD=root`
  - `MYR_RUN_TUI_MYSQL_INTEGRATION=1`
- cross-platform validation:
  - `cargo test --workspace --all-features --locked`
  - `cargo build --workspace --all-features --locked`
  - runs on `ubuntu-latest`, `macos-latest`, and `windows-latest`
- optional cross-platform keyring smoke check:
  - enabled when repository variable `MYR_CI_RUN_KEYRING_SMOKE=1`
  - command: `MYR_RUN_KEYRING_SMOKE=1 cargo test -p myr-adapters keyring_password_round_trip_when_enabled -- --nocapture`
- perf smoke trend guard:
  - policy file: `bench/perf-trend-policy.json`
  - command flag: `--trend-policy bench/perf-trend-policy.json`
- command:

```bash
cargo llvm-cov --workspace --all-features \
  --json --summary-only \
  --output-path target/coverage/summary.json \
  --fail-under-lines 85
```

Optional local integration commands:

```bash
MYR_DB_PASSWORD=root MYR_RUN_MYSQL_INTEGRATION=1 \
  cargo test -p myr-adapters --test mysql_integration -- --nocapture

# Same test suite against a local MariaDB instance.
# Example service: docker run --rm --name myr-mariadb \
#   -e MARIADB_ROOT_PASSWORD=root -p 33307:3306 mariadb:11.4
MYR_DB_PASSWORD=root MYR_RUN_MYSQL_INTEGRATION=1 \
  MYR_TEST_DB_HOST=127.0.0.1 MYR_TEST_DB_PORT=33307 MYR_TEST_DB_USER=root \
  cargo test -p myr-adapters --test mysql_integration -- --nocapture

MYR_DB_PASSWORD=root MYR_RUN_TUI_MYSQL_INTEGRATION=1 \
  MYR_TEST_DB_HOST=127.0.0.1 MYR_TEST_DB_PORT=33306 \
  MYR_TEST_DB_USER=root MYR_TEST_DB_DATABASE=myr_bench \
  cargo test -p myr-tui mysql_query_path_streams_rows_when_enabled -- --nocapture

MYR_RUN_KEYRING_SMOKE=1 \
  cargo test -p myr-adapters keyring_password_round_trip_when_enabled -- --nocapture

# Non-interactive CLI smoke checks.
MYR_DB_PASSWORD=root cargo run -p myr-app -- \
  query --host 127.0.0.1 --port 33306 --user root --database myr_bench \
  --sql "SELECT 1 AS health_check"

MYR_DB_PASSWORD=root cargo run -p myr-app -- \
  doctor --host 127.0.0.1 --port 33306 --user root --database myr_bench
```

Runtime behavior knobs (optional):
- `MYR_AUDIT_MAX_BYTES`: rotate `audit.ndjson` when file exceeds this size (default `5242880`).
- `MYR_AUDIT_MAX_ARCHIVES`: number of rotated audit files to keep (default `3`).

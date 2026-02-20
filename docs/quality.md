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
- line coverage threshold: `80%`
- MySQL integration coverage enabled with:
  - `MYR_DB_PASSWORD=root`
  - `MYR_RUN_MYSQL_INTEGRATION=1`
- gated TUI MySQL query-path integration:
  - `MYR_DB_PASSWORD=root`
  - `MYR_RUN_TUI_MYSQL_INTEGRATION=1`
- cross-platform validation:
  - `cargo test --workspace --all-features --locked`
  - `cargo build --workspace --all-features --locked`
  - runs on `ubuntu-latest`, `macos-latest`, and `windows-latest`
- command:

```bash
cargo llvm-cov --workspace --all-features \
  --json --summary-only \
  --output-path target/coverage/summary.json \
  --fail-under-lines 80
```

Optional local integration commands:

```bash
MYR_DB_PASSWORD=root MYR_RUN_MYSQL_INTEGRATION=1 \
  cargo test -p myr-adapters --test mysql_integration -- --nocapture

MYR_DB_PASSWORD=root MYR_RUN_TUI_MYSQL_INTEGRATION=1 \
  MYR_TEST_DB_HOST=127.0.0.1 MYR_TEST_DB_PORT=33306 \
  MYR_TEST_DB_USER=root MYR_TEST_DB_DATABASE=myr_bench \
  cargo test -p myr-tui mysql_query_path_streams_rows_when_enabled -- --nocapture
```

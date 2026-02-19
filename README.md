# Fast MySQL TUI Explorer (Rust)

Terminal-first MySQL/MariaDB schema and data explorer focused on speed, guided actions, and safe defaults.

## Status

M0-M3 backlog milestones are implemented for explorer/navigation, guided actions, pagination,
and benchmark/coverage gates. The detailed product backlog is in
`fast-mysql-tui-explorer-backlog.md`.

## Workspace Layout

- `app`: binary entrypoint
- `crates/core`: domain logic and shared state
- `crates/tui`: terminal UI components
- `crates/adapters`: external integrations (DB/export/fs)
- `docs`: architecture and contributor docs
- `bench`: benchmark and dataset tooling

## Getting Started

1. Install Rust via `rustup`.
2. Run `cargo build` from the repository root.
3. Run `cargo test` to verify baseline health.
4. Start the app with `cargo run -p myr-app`.

## Key Features

- Connection wizard with persisted profiles
- Schema explorer lanes for databases, tables, and columns
- Context-aware next actions in footer + command palette
- Safe mode confirmation for destructive SQL
- Table preview pagination:
  - Keyset pagination for detected `id` / `*_id` keys
  - OFFSET fallback when keyset is unavailable
- Export to CSV/JSON
- Benchmark runner + perf smoke checks

## MySQL Connection Notes

- Connection profiles in the TUI now attempt real MySQL connections via `mysql_async`.
- Passwords are read from the `MYR_DB_PASSWORD` environment variable.
- Schema/table loading and query execution use the live adapter when connected.
- Table preview now supports paging actions: keyset pagination on detected `id`/`*_id` columns with OFFSET fallback.

## Benchmark Quickstart

- Start local benchmark DB: `docker compose -f bench/docker-compose.yml up -d --wait`
- Run benchmark runner:
  - `MYR_DB_PASSWORD=root cargo run -p myr-app --bin benchmark -- --host 127.0.0.1 --port 33306 --user root --database myr_bench --seed-rows 50000`
- One-command setup/run/teardown:
  - `bench/scripts/run_benchmark.sh`

## Quality Gates

- Local baseline:
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test`
  - `cargo build`
- Coverage report:
  - `cargo llvm-cov --workspace --all-features --html --output-dir target/coverage/html`
- CI coverage gate:
  - minimum lines: `80%`
  - MySQL-backed integration tests enabled via `MYR_RUN_MYSQL_INTEGRATION=1`
  - see `.github/workflows/ci.yml`
- CI cross-platform validation:
  - test + build on `ubuntu-latest`, `macos-latest`, and `windows-latest`
- Run MySQL integration tests locally (optional):
  - `MYR_DB_PASSWORD=root MYR_RUN_MYSQL_INTEGRATION=1 cargo test -p myr-adapters --test mysql_integration`
- Additional quality docs:
  - `docs/quality.md`

## License

MIT (`LICENSE`).

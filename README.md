# Fast MySQL TUI Explorer (Rust)

Terminal-first MySQL/MariaDB schema and data explorer focused on speed, guided actions, and safe defaults.

## Status

Project bootstrap complete. The detailed product backlog is in `fast-mysql-tui-explorer-backlog.md`.

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

## License

MIT (`LICENSE`).

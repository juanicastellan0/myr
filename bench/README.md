# Bench

Benchmark and perf-regression tooling for local and CI checks.

## Local DB Harness

A local MySQL benchmark instance is defined in `bench/docker-compose.yml`.

Start it manually:

```bash
docker compose -f bench/docker-compose.yml up -d --wait
```

Stop and remove data:

```bash
docker compose -f bench/docker-compose.yml down -v
```

## Benchmark Runner

The benchmark runner is `myr-app` binary target `benchmark`:

```bash
export MYR_DB_PASSWORD=root
cargo run -p myr-app --bin benchmark -- \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --seed-rows 50000 \
  --sql "SELECT id, user_id, category, payload, created_at FROM events ORDER BY id LIMIT 50000" \
  --metrics-label local-run \
  --metrics-output target/perf/local-run.json
```

Reported metrics:
- `metric.connect_ms`
- `metric.first_row_ms`
- `metric.rows_streamed`
- `metric.stream_elapsed_ms`
- `metric.rows_per_sec`
- `metric.peak_memory_bytes` (Linux best effort via `/proc/self/status`)

Optional regression gates are built in:

```bash
cargo run -p myr-app --bin benchmark -- \
  --host 127.0.0.1 \
  --port 33306 \
  --user root \
  --database myr_bench \
  --seed-rows 10000 \
  --assert-first-row-ms 4000 \
  --assert-min-rows-per-sec 2000
```

`--metrics-output` writes a machine-readable JSON payload (including metadata + metrics) for
trend tracking in CI artifacts or local historical comparisons.

## One-command Local Run

`bench/scripts/run_benchmark.sh` boots MySQL, runs the benchmark, then tears down the DB:

```bash
bench/scripts/run_benchmark.sh
```

## One-command Dev Dataset

`scripts/dev-db-seed.sh` boots local MySQL and seeds an idempotent test dataset for manual TUI connection checks:

```bash
scripts/dev-db-seed.sh
```

## CI Perf Smoke

CI runs a benchmark smoke check in `.github/workflows/ci.yml` against a MySQL service with:
- `--seed-rows 10000`
- `--assert-first-row-ms 5000`
- `--assert-min-rows-per-sec 500`
- `--metrics-output target/perf/perf-smoke.json`

The workflow uploads `target/perf/perf-smoke.json` as an artifact (`perf-smoke-<run_id>`) so
perf trends can be tracked over time across CI runs.

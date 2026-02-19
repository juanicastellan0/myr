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
- command:

```bash
cargo llvm-cov --workspace --all-features \
  --json --summary-only \
  --output-path target/coverage/summary.json \
  --fail-under-lines 80
```

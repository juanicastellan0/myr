# Contributing

Thanks for contributing.

## Development setup

1. Install Rust (`rustup`).
2. Clone the repository.
3. Run `cargo build` and `cargo test` from the root.

## Pull request expectations

- Keep changes scoped and testable.
- Add or update tests for behavior changes.
- Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test`, and `cargo build` before opening a PR.
- Run coverage locally when your change affects core behavior:
  - `cargo llvm-cov --workspace --all-features --summary-only`
- Update docs when behavior or architecture changes.

## CI quality gates

- CI enforces formatting, clippy, tests, build, perf smoke checks, and coverage.
- Coverage gate currently requires at least `75%` line coverage and excludes
  `crates/tui/src/lib.rs` from threshold evaluation.
- See `.github/workflows/ci.yml` and `docs/quality.md`.

## Commit style

Use clear, imperative commit messages describing what changed.

## Reporting issues

Open an issue with:
- expected behavior
- actual behavior
- reproduction steps
- environment details

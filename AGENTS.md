# AGENTS

Operational guidance for coding agents working in this repository.

## Scope

- Applies to the whole repository.
- Prefer small, testable changes.
- Do not revert unrelated local changes in the working tree.

## Repository Map

- `app/`: binary entrypoints (`myr-app`, benchmark runner)
- `crates/core/`: domain logic (actions, pagination, guards, profiles)
- `crates/adapters/`: MySQL/export adapters
- `crates/tui/`: terminal UI and interaction logic
- `bench/`: local MySQL benchmark harness and scripts
- `docs/`: project documentation
- `scripts/dev-db-seed.sh`: one-command dev dataset seeding for manual testing

## Preferred Workflow

1. Read relevant code and docs before editing.
2. Implement the smallest change that solves the user request.
3. Add or update tests for behavior changes.
4. Update docs when user-visible behavior changes.
5. Run validation commands and report what passed/failed.

## Validation Commands

Run from repository root when possible:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

If the local Rust toolchain is unavailable, use the Docker workflow already used in this repo.

## Database and Manual Test Setup

Seed local test data:

```bash
scripts/dev-db-seed.sh
```

Then set:

```bash
export MYR_DB_PASSWORD=root
```

Manual test details and expected outcomes are in `docs/manual-testing.md`.

## Integration Test Toggles

- Adapter integration: `MYR_RUN_MYSQL_INTEGRATION=1`
- TUI MySQL query-path integration: `MYR_RUN_TUI_MYSQL_INTEGRATION=1`

## Documentation Sync

When behavior changes, keep these in sync:

- `README.md` for user-facing quickstart and features
- `docs/quality.md` for gate/test commands
- `docs/manual-testing.md` for QA checklist
- `CONTRIBUTING.md` for contributor expectations

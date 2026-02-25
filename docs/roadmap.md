# Roadmap

## Completed Baseline (M0-M8)

All M0-M8 milestones are complete (bootstrap, explorer, guided actions, pagination/perf, reliability, query UX, security, and power-user workflows).

## Deep Analysis Snapshot (2026-02-24)

### Current State

- Product scope is broad and coherent: connect -> explore schema -> run/shape queries -> page/search/export -> recover from failures.
- Delivery quality gates are in place: fmt, clippy, tests, build, coverage gate, perf smoke, and cross-platform CI.
- Test volume is solid (`129` Rust tests), with adapter and TUI MySQL-gated integration paths.

### Engineering Shape

- Codebase size is moderate (`~10.9k` Rust LOC) and still moving quickly.
- Remaining concentration risk is in a handful of files:
  - `crates/tui/src/app_logic/navigation.rs` (673)
  - `crates/tui/src/app_logic/runtime.rs` (653)
  - `crates/tui/src/app_logic/query_actions.rs` (551)
  - `app/src/bin/benchmark.rs` (620)
  - `crates/core/src/schema_cache.rs` (442)
- TUI rendering and core action engine were recently decomposed, reducing single-file blast radius.

### Key Gaps and Risks

- UX gaps:
  - Results table readability still relies on text rendering; no horizontal scroll/frozen headers.
  - Command palette filter is substring-based only (no fuzzy ranking/aliases).
  - Profile/bookmark management is functional but lacks a dedicated management UI.
- Reliability gaps:
  - `unwrap()` remains in adapter row conversion path and should be removed from runtime code.
  - Recovery/retry flows are strong but failure-injection coverage can be expanded.
- Docs/architecture gaps:
  - Architecture notes and action engine extension docs are still missing (`docs/README.md` planned additions).
- Platform/distribution gaps:
  - Release artifacts are x86_64-only; no arm64 distribution target yet.
  - CI integration matrix focuses on MySQL; MariaDB compatibility is not yet gated.

### Priorities

- P0: Maintainability and reliability hardening.
- P1: Query/results UX and discoverability.
- P2: Compatibility/distribution expansion.

## M9: UX Clarity and Discoverability

- [x] Results table v2:
  - Horizontal scroll and viewport indicator.
  - Sticky header row + clearer selected-row/selected-column emphasis.
  - Better width strategy for long text/JSON cells.
- [x] Schema explorer v2:
  - [x] Filter-as-you-type for database/table/column lists.
  - [x] Optional compact/full metadata view for columns.
- [x] Query editor v2:
  - Explicit cursor row/column ruler and active SQL region emphasis.
  - Improved multiline ergonomics for long statements.
- [x] Command palette search v2:
  - Fuzzy match with ranking by score + recency + context.
  - Action aliases/keywords (`ddl`, `export`, `bookmark`, etc.).
- [x] Profile/bookmark manager screen:
  - [x] List/select/delete.
  - [x] Rename entries.
  - [x] Mark default profile and quick reconnect target.

## M10: Reliability and Safety Hardening (Post-M8)

- [x] Remove panic paths from runtime code (`unwrap` -> typed error propagation).
- [x] Expand reconnect/cancel/timeout failure-injection tests.
- [x] Tighten read-only guard coverage for edge SQL patterns (transaction + mixed statements).
- [x] Add audit trail rotation/retention options with safe defaults.
- [x] Add explicit health diagnostics command/action (`connection + schema + query smoke`).

## M11: Architecture and Dev Velocity

- [ ] Split remaining large files by bounded context:
  - TUI `runtime`, `navigation`, `query_actions`
  - core `schema_cache`
  - app `benchmark`
- [ ] Move TUI state/data model types out of `crates/tui/src/lib.rs` into dedicated modules.
- [ ] Add architecture documentation:
  - Event loop and message flow.
  - Worker lifecycle (connect/query threads + cancellation).
  - Action engine invocation path.
- [ ] Add "Action Engine Extension Guide" (new action checklist + test strategy).

## M12: Quality and Compatibility Expansion

- [ ] Add MariaDB integration lane in CI.
- [ ] Add optional keyring smoke checks for linux/macos/windows runners.
- [ ] Add rendering snapshot-style tests for key panes/popups.
- [ ] Raise coverage gate from `80%` to `85%` once flaky/low-value tests are addressed.
- [ ] Define benchmark trend guard policy (baseline file + tolerance windows).

## M13: Distribution and Adoption

- [ ] Build and publish arm64 artifacts (`linux-aarch64`, `macos-aarch64`).
- [ ] Add install channels (Homebrew tap and Scoop manifest).
- [ ] Add non-interactive CLI entrypoints for scripting:
  - `myr-app query --sql ...`
  - `myr-app export --format ...`
  - `myr-app doctor`
- [ ] Add config/profile migration helper for forward-compatible upgrades.

## Next Up (Proposed Execution Order)

- [ ] Add architecture notes in `docs/architecture.md`.
- [ ] Split `app/src/bin/benchmark.rs` into parser/runner/report modules.
- [ ] Refactor benchmark metrics writer (`app/src/bin/benchmark.rs`) to satisfy clippy gate.

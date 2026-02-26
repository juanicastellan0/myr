# Roadmap

Last updated: 2026-02-26

## Mission

`myr` should be the fastest and safest terminal workflow for MySQL/MariaDB:

- connect quickly
- inspect schema confidently
- execute and iterate on SQL safely
- export and automate reliably
- recover cleanly from failures

## Baseline Status (M0-M13)

Milestones M0-M13 are complete (core TUI workflows, guided actions, pagination, reliability hardening, benchmark gates, install channels, non-interactive CLI, and architecture decomposition).

## Deep Analysis Snapshot (2026-02-26)

### Codebase shape

- Rust source files: `74`
- Rust LOC (`app` + `crates`): `15,405`
- Test functions (`#[test]` + `#[tokio::test]`): `185`

Crate distribution:

| Area | Files | LOC | Tests |
| --- | ---: | ---: | ---: |
| `app` | 7 | 1,845 | 28 |
| `crates/core` | 24 | 4,405 | 58 |
| `crates/adapters` | 4 | 1,035 | 12 |
| `crates/tui` | 39 | 8,120 | 87 |

Largest current hotspots (non-generated):

- `app/src/main.rs` (865)
- `crates/core/src/profiles.rs` (572)
- `crates/tui/src/app_logic/navigation/manager_interactions.rs` (545)
- `crates/adapters/src/mysql.rs` (522)
- `crates/tui/src/app_logic/navigation/schema_traversal.rs` (463)
- `crates/tui/src/lib_helpers.rs` (446)
- `crates/core/src/safe_mode.rs` (413)
- `crates/tui/src/rendering/pane_results.rs` (380)
- `crates/core/src/audit_trail.rs` (379)

Testing concentration:

- `crates/tui/src/tests.rs` is `1,873` LOC (largest single file in repo).

### Current strengths

- Clear crate boundaries and dependency direction.
- Strong baseline gates (fmt, clippy, test, build, coverage floor, perf smoke).
- Resilience patterns already in place (retry, auto-reconnect, timeout/cancel paths).
- Feature depth is high for a terminal client: safe mode, read-only guard, actions engine, pagination, bookmarks/profiles manager, non-interactive commands, benchmarks.

### Key gaps and risks (evidence-driven)

1. Export scalability risk:
   - CLI export path materializes all rows in memory (`collect_query_rows` in `app/src/main.rs`).
   - TUI export only writes buffered rows, not full result sets (`app_logic/navigation/export.rs`).
2. Data fidelity gap:
   - Query rows are normalized to `String`; typed values and `NULL` semantics are lossy (`crates/adapters/src/mysql.rs`, `app/src/main.rs` JSON output path).
3. Partial action implementation:
   - `CopyCell`/`CopyRow` actions only write status text; no actual clipboard integration (`query_actions/action_dispatch.rs`).
4. Schema scale risk:
   - Schema loading is whole-catalog fetch from `information_schema`; large instances may be slow/heavy (`crates/adapters/src/mysql.rs`, `schema_cache/service.rs`).
5. Safety parser limits:
   - Safe mode uses heuristic tokenization, not SQL AST parsing (`crates/core/src/safe_mode.rs`).
6. Maintainability concentration:
   - App entrypoint and manager interactions remain large and multi-responsibility.
   - TUI tests are highly centralized in one file, reducing locality and review clarity.
7. Audit/privacy control gap:
   - SQL text is persisted by default in audit trail; no built-in redaction policies (`crates/core/src/audit_trail.rs`).

## Priority Framework

- `P0`: correctness/safety/reliability regressions and data-loss risks.
- `P1`: scale/perf bottlenecks and workflow completeness gaps.
- `P2`: maintainability, distribution, and adoption improvements.

## Issue Ledger (New)

| ID | Pri | Type | Summary | Evidence |
| --- | --- | --- | --- | --- |
| RDM-001 | P0 | Reliability | Stream CLI export to disk without full in-memory capture. | `app/src/main.rs::collect_query_rows` |
| RDM-002 | P0 | UX/Data | Add explicit "buffered export" vs "re-run full export" modes in TUI. | `crates/tui/src/app_logic/navigation/export.rs` |
| RDM-003 | P0 | Data | Introduce typed query values (string/number/bool/null/bytes/time) across backend/core/CLI. | `crates/adapters/src/mysql.rs`, `query_runner` |
| RDM-004 | P1 | UX | Implement real clipboard integration for copy actions (OS-aware fallback). | `query_actions/action_dispatch.rs` |
| RDM-005 | P1 | Perf | Add lazy schema loading by database/table, plus targeted refresh. | `mysql.rs` schema fetch + `schema_cache` |
| RDM-006 | P1 | Safety | Harden safe-mode parsing for edge SQL (comments/quotes/CTE/procedural statements). | `core/safe_mode.rs` |
| RDM-007 | P1 | Architecture | Split `app/src/main.rs` into subcommand modules (parser/runner/output). | `app/src/main.rs` size/responsibility |
| RDM-008 | P1 | Testability | Split `crates/tui/src/tests.rs` by feature domain + integration toggles. | `crates/tui/src/tests.rs` |
| RDM-009 | P1 | Architecture | Reduce `TuiApp` state coupling with bounded sub-state structs. | `state/app.rs` |
| RDM-010 | P1 | Security | Add audit redaction mode and per-profile audit controls. | `core/audit_trail.rs` + runtime append path |
| RDM-011 | P2 | Perf | Replace per-query thread+runtime spin-up with pooled async executor model. | `lib_helpers` worker spawn pattern |
| RDM-012 | P2 | DX | Add command-level benchmarks for CLI query/export/doctor regressions. | `bench` currently query benchmark-focused |

## Milestones

## M14: Data Fidelity and Export at Scale

Goal: make query/export paths safe for large datasets and preserve value semantics.

Targets: `RDM-001`, `RDM-002`, `RDM-003`

### Deliverables

- [ ] Replace CLI `collect_query_rows` with streaming writers for CSV/JSON/JSONL(+gzip).
- [ ] Add `--max-rows` and `--progress` options for non-interactive `query`/`export`.
- [ ] Introduce typed cell representation in core (`QueryValue`) and keep `NULL` as null, not `"NULL"`.
- [ ] Keep compatibility mode for stringified output (`--stringify-values`) to avoid breaking scripts.
- [ ] In TUI, separate:
  - buffered export (current results ring buffer),
  - full export (re-run SQL with streaming export worker and progress/cancel).
- [ ] Document memory behavior and limits in `README.md` + `docs/manual-testing.md`.

### Acceptance Criteria

- Exporting 1M+ rows does not require full row materialization in application memory.
- CLI JSON output can emit typed JSON values.
- TUI clearly reports export mode and row counts.
- New integration tests cover large-row streaming and cancel behavior.

### Validation

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo build`
- Add benchmark scenario: export throughput and memory envelope.

## M15: Schema Scale and Result Navigation

Goal: keep interactive workflows responsive for large/complex database catalogs.

Targets: `RDM-005`

### Deliverables

- [ ] Add incremental schema loading:
  - list databases first,
  - fetch tables on database selection,
  - fetch columns/relationships on table selection.
- [ ] Add TTL + manual refresh controls at each scope.
- [ ] Add schema fetch telemetry to status line and optional debug output.
- [ ] Add defensive limits (max tables/columns loaded per request with user messaging).
- [ ] Improve results table ergonomics:
  - dedicated cell-inspect mode for long values,
  - optional wrapped mode for selected row details.

### Acceptance Criteria

- Initial connect no longer requires loading full global schema catalog.
- Schema lane interactions remain responsive on large datasets.
- Relationship browsing still works with lazy loading.

### Validation

- New adapter integration tests for scoped schema fetch behavior.
- Manual test cases added for large-schema navigation scenarios.

## M16: Reliability and Safety Hardening v2

Goal: reduce false positives/negatives in safety checks and make failure handling deterministic.

Targets: `RDM-006`, `RDM-011`

### Deliverables

- [ ] Expand safe-mode parser coverage for:
  - escaped quotes/backticks/comments,
  - CTEs and multi-line statements,
  - transaction/session mutation edge cases.
- [ ] Add optional strict mode (block unknown statements by default, explicit allow override).
- [ ] Normalize runtime error taxonomy (connection/auth/network/query/timeout/cancel).
- [ ] Replace repeated ad-hoc worker runtime spin-up with a pooled execution model.
- [ ] Add failure-injection tests for reconnect + cancel races.

### Acceptance Criteria

- Safe-mode regressions are covered by high-signal tests (including pathological SQL samples).
- Retry/reconnect behavior is deterministic and bounded.
- Query cancellation always terminates worker path cleanly.

### Validation

- Add targeted property/fuzz tests for SQL splitting and classification.
- Add regression matrix for transient error families.

## M17: Architecture and Test System Refinement

Goal: improve contributor velocity and reduce review/debug friction.

Targets: `RDM-007`, `RDM-008`, `RDM-009`

### Deliverables

- [ ] Split `app/src/main.rs` by responsibility:
  - command parsing,
  - connection resolution,
  - command runners,
  - rendering/output helpers.
- [ ] Decompose `TuiApp` state into explicit sub-structs (`ConnectionState`, `QueryState`, `SchemaState`, `ManagerState`, etc.).
- [ ] Split `crates/tui/src/tests.rs` by feature module and shared fixture utilities.
- [ ] Add coding standards guardrails:
  - max file size advisory,
  - module ownership docs,
  - change-placement guide updates.

### Acceptance Criteria

- No single production file above 700 LOC.
- TUI tests are grouped by runtime domain and easier to navigate.
- New contributors can locate behavior by feature folder with minimal cross-file hops.

### Validation

- Update `docs/architecture.md` and `docs/README.md` with new module map.
- Ensure no behavior regressions in existing snapshot and integration tests.

## M18: Security, Privacy, and Ops Controls

Goal: strengthen production posture for teams using `myr` in sensitive environments.

Targets: `RDM-010`

### Deliverables

- [ ] Add audit redaction mode:
  - configurable SQL masking,
  - optional hash-only statement recording,
  - per-profile audit enable/disable.
- [ ] Add keyring diagnostics command (`doctor --check-keyring`) with explicit failure reporting.
- [ ] Add profile-level policy flags for:
  - disable exports,
  - disable clipboard copy,
  - enforce read-only mode.
- [ ] Add docs for secure deployment defaults.

### Acceptance Criteria

- Sensitive SQL can be masked while preserving operational telemetry.
- Operators can validate keyring behavior non-interactively.
- Policy flags are enforced consistently in TUI and CLI.

### Validation

- Add tests for redaction output and policy enforcement.
- Update `README.md`, `docs/manual-testing.md`, and `docs/quality.md`.

## M19: CLI and Ecosystem Maturity

Goal: make `myr` easier to automate and operate in CI/tooling pipelines.

Targets: `RDM-004`, `RDM-012`

### Deliverables

- [ ] Add machine-readable output mode for `doctor` (`--json`).
- [ ] Add `query`/`export` exit-code contract documentation by failure class.
- [ ] Add command-level benchmark suite (query/export/doctor) with trend policies.
- [ ] Implement real clipboard integration with capability detection and graceful fallback.
- [ ] Improve release/distribution docs for cross-arch usage and verification.

### Acceptance Criteria

- CI users can parse health status via structured output.
- Automation scripts can rely on stable exit-code behavior.
- Clipboard actions perform real copy where supported.

### Validation

- CLI smoke tests include structured-output assertions.
- Benchmark artifacts include per-command metrics.

## Cross-Milestone Quality Gates

Every milestone completion requires:

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo build`
- Roadmap/doc sync:
  - `README.md`
  - `docs/quality.md`
  - `docs/manual-testing.md`
  - `CONTRIBUTING.md` (when contributor workflow changes)

## Proposed Execution Order

1. M14 (data/export correctness and scale)
2. M15 (schema scale + navigation responsiveness)
3. M16 (safety and runtime determinism)
4. M17 (architecture and test maintainability)
5. M18 (security/privacy controls)
6. M19 (CLI/ops ecosystem polish)

## Definition of Done for This Roadmap Revision

- Old completed-only roadmap replaced with forward-looking, issue-driven plan.
- New issues are traceable to current code evidence.
- Milestones include acceptance criteria and validation strategy, not only feature bullets.

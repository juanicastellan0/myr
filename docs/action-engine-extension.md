# Action Engine Extension Guide

This guide defines the expected workflow for adding a new action to the action engine, including
implementation touchpoints and test coverage expectations.

## Runtime Boundary

Keep action design aligned with the current architecture split:

- `crates/core/src/actions_engine/*` is pure domain logic:
  - action catalog and IDs
  - context-gated enablement
  - ranking and recency boost
  - mapping `ActionId` to `ActionInvocation`
- `crates/tui/src/app_logic/query_actions/action_dispatch.rs` is the runtime side-effect boundary:
  - executes SQL
  - mutates pane/view state
  - triggers export/bookmark/search/diagnostics workflows

Add behavior in the core engine first. Add side effects in TUI only after invocation shape is stable.

## New Action Checklist

Use this checklist for every new action:

1. Add catalog identity and metadata.
   - Add the new `ActionId` variant in `crates/core/src/actions_engine/catalog.rs`.
   - Add an `ActionDefinition` entry in `ACTIONS` with clear title/description text.
2. Choose invocation contract.
   - Reuse an existing `ActionInvocation` variant when possible.
   - If needed, add a new variant in `crates/core/src/actions_engine/invocation.rs`.
3. Define enablement rules.
   - Add the `ActionId` branch in `crates/core/src/actions_engine/enablement.rs`.
   - Keep enablement deterministic and tied to `ActionContext`.
4. Define ranking behavior.
   - Add a base score in `crates/core/src/actions_engine/ranking.rs`.
   - Position score relative to adjacent actions in the same pane/context.
5. Implement engine invocation mapping.
   - Add the `ActionId` branch in `crates/core/src/actions_engine/engine.rs::invoke`.
   - Prefer existing SQL builder helpers in `crates/core/src/sql_generator.rs` when generating SQL.
6. Wire TUI side effects (if invocation is new).
   - Handle the new invocation in `crates/tui/src/app_logic/query_actions/action_dispatch.rs`.
   - Preserve existing status-line and pane-state conventions.
7. Extend context shape only when required.
   - If the action needs new state, add fields to `ActionContext` in `catalog.rs`.
   - Populate fields in `crates/tui/src/app_logic/query_actions/action_dispatch.rs::action_context`.
8. Update palette discoverability.
   - Add aliases in `crates/tui/src/app_logic/navigation/palette_search.rs` for user-facing actions.
9. Update docs and roadmap.
   - Update user-facing docs if the action changes user-visible behavior.
   - Keep `docs/roadmap.md` checkboxes in sync when the milestone task is complete.

## Test Strategy

### Core Unit Tests (Required)

Add tests in `crates/core/src/actions_engine/tests.rs`:

- Enablement and invocation happy path:
  - Build an `ActionContext` where the action should be enabled.
  - Assert `ActionsEngine::invoke` returns the expected `ActionInvocation`.
- Disabled-path guard:
  - Build a context where the action should be disabled.
  - Assert `invoke` returns `ActionEngineError::ActionDisabled(...)`.
- Ranking placement:
  - Assert `rank_top_n` includes the action for relevant contexts.
  - Assert ordering when priority should beat or trail nearby actions.

### TUI Behavior Tests (Required For New Invocation Or Side Effect)

Add tests in `crates/tui/src/tests.rs`:

- Invocation application:
  - Assert `TuiApp::apply_invocation` mutates state, pane, and status line as expected.
- Context plumbing:
  - If enablement depends on new context fields, assert `action_context()` sets them correctly in
    representative pane/runtime states.
- Palette routing:
  - If aliases are added, assert `palette_entries()` surfaces the action for alias/fuzzy queries.

### Manual/Integration Coverage (When DB Or Workflow Behavior Changes)

- Seed local dataset with `scripts/dev-db-seed.sh`.
- Export `MYR_DB_PASSWORD=root`.
- Run through action flow in TUI and verify expected status/error paths.
- If behavior is user-visible, add a concise scenario to `docs/manual-testing.md`.

## Validation Gate

Before opening a PR, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

If any command fails, fix or document the blocker before merging.

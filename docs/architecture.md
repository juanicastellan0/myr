# Architecture

This document captures the current runtime architecture for `myr`, with emphasis on:

- Workspace crate boundaries and dependency direction
- Event loop and message flow
- Worker lifecycle (connect/query workers + cancellation)
- Action engine invocation path
- TUI module boundaries and state ownership rules

## Workspace Layering And Dependency Direction

`myr` is organized as a layered workspace:

```text
app (entrypoints + CLI orchestration)
  -> myr-tui (interactive runtime + rendering)
  -> myr-adapters (MySQL + export implementations)
  -> myr-core (domain contracts, policies, and services)
```

Dependency rules reflected in `Cargo.toml` files:

- `myr-core` is domain-only and does not depend on `myr-tui` or `myr-adapters`.
- `myr-adapters` depends on `myr-core` contracts to provide concrete MySQL/export behavior.
- `myr-tui` depends on both `myr-core` and `myr-adapters` because it orchestrates domain actions and IO-backed workflows.
- `myr-app` is composition-only: it wires runtime modes and delegates to TUI/core/adapters.

Practical implication: keep policy/decision logic in `myr-core`, keep protocol/driver code in `myr-adapters`, and keep user-interaction orchestration in `myr-tui`.

## Runtime Topology

Primary crates involved in runtime flow:

- `app/src/main.rs`: binary entrypoint, calls `myr_tui::run()`
- `crates/tui/src/lib.rs`: terminal setup/restore + outer render/event loop
- `crates/tui/src/app_logic/*`: message handling, navigation, query/connect orchestration
- `crates/tui/src/lib_helpers.rs`: key mapping and worker functions
- `crates/core/src/actions_engine/*`: action catalog, ranking, enablement, invocation mapping
- `crates/core/src/query_runner.rs`: streaming query loop + cancellation contract

## TUI Module Boundaries

`crates/tui/src/lib.rs` is intentionally thin and delegates to four module groups:

- `state/*`: `TuiApp` and supporting enums/data (`Msg`, `Pane`, pagination/runtime/wizard types)
- `app_logic/*`: message handling and state transitions
  - `runtime/*`: top-level dispatch (`handle`) plus connect/query polling lifecycles
  - `navigation/*`: pane movement, schema traversal/filtering, manager interactions, palette/results navigation
  - `query_actions/*`: action dispatch, guarded query execution, pagination transitions, error panel logic
  - `input.rs`: pane-specific text/editing handlers
- `rendering/*`: pure frame rendering (chrome, panes, overlays)
- `lib_helpers.rs`: key mapping, worker entrypoints, and small shared helpers

Design constraints used by this layout:

- Rendering modules are read-only over `TuiApp` state and do not perform IO.
- Mutating behavior should enter through `TuiApp::handle(Msg)` and stay in `app_logic/*`.
- Background work must not mutate `TuiApp` directly; workers return outcomes over channels.

## State Ownership And Concurrency Rules

`TuiApp` (in `state/app.rs`) is the single mutable owner of runtime state on the UI thread.
Concurrency is explicit and message-like:

- input/tick events become `Msg` values
- `Msg` is handled synchronously in `TuiApp::handle`
- connect/query workers run on dedicated threads and return a single `ConnectWorkerOutcome`/`QueryWorkerOutcome` over `mpsc`
- cancellation uses `CancellationToken`, which is the only cross-thread control signal during query streaming

This keeps race risk low: worker threads do not hold references to `TuiApp` fields.

## Runtime Modes And Entrypoints

`app/src/main.rs` provides four runtime modes:

- default/no subcommand: launches `myr_tui::run()`
- `query`: non-interactive SQL execution to JSONL output
- `export`: non-interactive export to CSV/JSON/JSONL (+ gzip variants)
- `doctor`: connection/schema/query smoke diagnostics

All non-interactive modes reuse the same core/adapters services (`ConnectionManager`, `SchemaCacheService`, `QueryBackend`).
The benchmark binary (`app/src/bin/benchmark.rs`) is separate and split by concern (`parser`, `runner`, `report`, `model`) to avoid mixing CLI parsing, measurement, and policy checks.

## Event Loop And Message Flow

The UI runs a single-threaded loop in `crates/tui/src/lib.rs::run_loop`:

1. Render current `TuiApp` state via `render(frame, &app)`.
2. Poll keyboard input with timeout based on `TICK_RATE`.
3. Convert key events to domain messages (`Msg`) through `map_key_event`.
4. Dispatch messages through `TuiApp::handle`.
5. Emit periodic `Msg::Tick` and process runtime polling/heartbeat work.

Message dispatch is centralized in `crates/tui/src/app_logic/runtime/handle.rs::handle`, which:

- gates special modes first (exit confirmation, error panel, results search mode)
- routes the remaining message through pane-aware handlers (`submit`, `navigate`, `connect`, etc.)
- updates state only through `TuiApp` methods

High-level flow:

```text
crossterm event::poll/read
  -> map_key_event(KeyEvent) -> Msg
  -> TuiApp::handle(Msg)
  -> mutate app state / spawn worker / queue side effects
  -> next render tick shows updated state
```

`Msg::Tick` is also the synchronization point for background work:

- `poll_connect_result()` checks connect worker channel
- `poll_query_result()` checks query worker channel
- spinner/status lines are refreshed while work is in flight

## Worker Lifecycle (Connect/Query + Cancellation)

Background work is isolated in short-lived threads, each returning one outcome through `std::sync::mpsc`.

### Connect Worker Lifecycle

Start path:

- user input triggers `Msg::Connect`
- `TuiApp::connect*` builds a `ConnectionProfile`
- `start_connect_with_profile` creates channel + spawns a thread
- spawned thread calls `run_connect_worker(profile)`

Execution details (`run_connect_worker`):

- creates a single-thread Tokio runtime
- performs `ConnectionManager::connect` with `CONNECT_TIMEOUT`
- performs disconnect cleanup warning capture
- loads database names through `SchemaCacheService::list_databases` (same timeout policy)
- returns `ConnectWorkerOutcome::{Success|Failure}`

Completion path:

- `Msg::Tick` -> `poll_connect_result()`
- applies connected state (`apply_connected_profile`) or opens error panel
- supports auto-reconnect retries (`ConnectIntent::AutoReconnect`, bounded by `AUTO_RECONNECT_LIMIT`)

### Query Worker Lifecycle

Start path:

- query action resolves to SQL (`execute_sql_with_guard` -> `start_query`)
- `start_query_internal` resets transient state, stores inflight SQL, sets results pane active
- creates `CancellationToken` + channel, then spawns worker thread
- worker calls `run_query_worker(backend, sql, cancellation)`

Execution details (`run_query_worker`):

- builds single-thread Tokio runtime
- executes `QueryRunner::execute_streaming` under `QUERY_TIMEOUT`
- streams rows into bounded `ResultsRingBuffer`
- on timeout: cancels token and returns failure

Cancellation behavior:

- cancel input (`Ctrl+C` / cancel action) triggers `Msg::CancelQuery`
- if query worker is active, app calls `cancellation.cancel()`
- `QueryRunner` checks token between row pulls, calls backend `stream.cancel()`, and returns `was_cancelled = true`

Completion path:

- `Msg::Tick` -> `poll_query_result()`
- consumes `QueryWorkerOutcome::{Success|Failure}`
- success: publishes buffered rows, pagination metadata, status/audit event
- failure: audit + retry logic (transient retry and optional auto-reconnect replay)

## Action Engine Invocation Path

The action engine is a pure domain service in `crates/core/src/actions_engine`.
For extension workflow and required tests, see `docs/action-engine-extension.md`.

### Discovery And Ranking

- `TuiApp::action_context()` maps UI/runtime state to `ActionContext`
- UI surfaces request ranked actions with `ActionsEngine::rank_top_n(...)`
  - footer shortcuts (`1..7`) in `rendering/chrome.rs`
  - command palette list in `app_logic/navigation/palette.rs`
- ranking combines:
  - static context score (`ranking.rs`)
  - enablement filtering (`enablement.rs`)
  - recency boost (`engine.rs`)

### Invocation

Invocation flow:

```text
key/palette selection
  -> TuiApp::invoke_action(action_id)
  -> ActionsEngine::invoke(action_id, context)
  -> ActionInvocation enum
  -> TuiApp::apply_invocation(...)
  -> side effect (run SQL, paginate, export, navigate, diagnostics, etc.)
```

`ActionsEngine::invoke` translates an `ActionId` into a concrete `ActionInvocation`:

- SQL-generating actions return `ActionInvocation::RunSql(...)`
- navigation/workflow actions return typed invocations (`OpenView`, `SearchBufferedResults`, etc.)
- invalid context returns typed `ActionEngineError`

`TuiApp::apply_invocation` is the boundary from domain intent to runtime side effects, including:

- guarded SQL execution (`execute_sql_with_guard`)
- pagination transitions
- export/bookmark workflows
- health diagnostics

## Change Placement Guide

When adding behavior, prefer these seams:

1. New keybinding/input: `lib_helpers::map_key_event` -> `Msg` -> `app_logic/runtime/handle.rs`.
2. Pane navigation/traversal behavior: relevant `app_logic/navigation/*` module.
3. New domain action: `crates/core/src/actions_engine/*`, then map runtime effect in `apply_invocation`.
4. New async worker step: spawn/poll flow in `app_logic/runtime/*` and worker body in `lib_helpers.rs`.
5. Rendering-only tweak: `rendering/*` without mutating runtime state.

## Practical Tracing Checklist

When debugging a runtime behavior:

1. Verify `map_key_event` emits the expected `Msg`.
2. Trace `TuiApp::handle` branch for that `Msg`.
3. For async behavior, follow worker spawn (`start_connect_with_profile` or `start_query_internal`) and the matching `poll_*_result`.
4. For action behavior, inspect `action_context` -> `ActionsEngine::invoke` -> `apply_invocation`.

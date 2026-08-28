# Architecture

This document captures the current runtime architecture for `myr`, with emphasis on:

- Workspace crate boundaries and dependency direction
- Event loop and message flow
- Worker lifecycle (connect/query workers + cancellation)
- Action engine invocation path
- TUI module boundaries and state ownership rules

## Workspace Layering And Dependency Direction

`myr` is organized around a shared application actor:

```text
myr-core
   ↑
myr-application ← myr-adapters
   ↑                  ↑
myr-tui          app / gui-app
myr-gui
```

Dependency rules reflected in `Cargo.toml` files:

- `myr-core` is domain-only and does not depend on `myr-tui` or `myr-adapters`.
- `myr-application` owns UI-independent commands, events, snapshots, safe mode/read-only enforcement, scoped schema state, bounded results, cancellation, export state, and profile mutations.
- `myr-adapters` implements the application ports with `mysql_async` and also retains low-level CLI/export adapters.
- `myr-gui` and the composed TUI consume `ApplicationHandle`; presentation state such as panes, focus, scroll, tabs, and themes never enters `AppSnapshot`.
- `myr-app` and `gui-app` are composition roots. They create one Tokio runtime, mount the MySQL factory, and pass a handle to their presentation.

Practical implication: keep policy in `myr-core`, workflows in `myr-application`, protocol code in `myr-adapters`, and presentation-only decisions in TUI/GUI.

## Shared Actor Runtime

`ApplicationHandle` sends `AppCommand` values over a bounded Tokio channel and exposes a watch snapshot plus broadcast events. Every connection, schema, query, and export operation receives a monotonically increasing `OperationId`; worker responses whose ID is no longer active are ignored.

Query workers stream typed rows in batches no faster than 10 Hz. The actor retains the newest 2,000 rows and publishes `rows_seen`, `rows_buffered`, and `truncated`. Cancellation is cooperative through `CancellationToken`. Full-query exports accept read-only SQL only, write to `<destination>.part`, report rows/bytes, remove partials on error/cancel, and rename only after completion.

Schema loading is scope-directed: connect calls only `list_databases`; database selection calls `list_tables`; table selection calls `list_columns` and `list_relationships`. `SchemaCacheService` independently caches each scope with TTL and targeted invalidation.

Profile writes use a sidecar filesystem lock, reload-under-lock mutation, temporary-file sync, and atomic replacement. `ConnectionProfile` contains password-source metadata but no password value.

## Runtime Topology

Primary crates involved in runtime flow:

- `app/src/main.rs`: CLI/TUI composition root; creates the shared actor and calls `myr_tui::run_with_application(...)`
- `gui-app/src/main.rs`: native GUI composition root for the `myr-gui` binary
- `crates/gui/src/*`: Iced presentation, GUI-only preferences, and headless UI tests
- `crates/tui/src/lib.rs`: terminal setup/restore + outer render/event loop over an `ApplicationHandle`
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

`TuiApp` (in `state/app.rs`) is the single mutable owner of terminal presentation state. The shared application actor owns connection, schema, query, result, export, and profile workflow state. Concurrency is explicit and message-like:

- input/tick events become `Msg` values
- `Msg` is handled synchronously in `TuiApp::handle`
- production actions become `AppCommand` values sent through `ApplicationHandle`
- each tick reads the latest `AppSnapshot`; operation IDs reject stale worker responses
- cancellation is an application command backed by a shared `CancellationToken`

The older direct worker path remains available only for the demo/compatibility harness used by existing TUI tests; composed production entrypoints always provide an application handle.

## Runtime Modes And Entrypoints

`app/src/main.rs` provides four runtime modes:

- default/no subcommand: composes the actor and launches `myr_tui::run_with_application(...)`
- `query`: non-interactive SQL execution to JSONL output
- `export`: non-interactive export to CSV/JSON/JSONL (+ gzip variants)
- `doctor`: connection/schema/query smoke diagnostics

`query` and `export` drive the same `AppCommand`/`AppEvent` contract as both user interfaces. `doctor` deliberately uses the low-level adapters so it can diagnose individual connection, schema, and query checks.
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

- the production TUI applies the newest `AppSnapshot`
- the compatibility harness polls its legacy connect/query worker channels
- spinner/status lines are refreshed while work is in flight

## Legacy TUI Worker Lifecycle (Compatibility Harness)

The following short-lived thread path is retained for demo behavior and regression tests. It is not used by `myr-app`; production connect/query/export work runs as cancelable Tokio tasks inside `myr-application`.

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

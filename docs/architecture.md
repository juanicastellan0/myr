# Architecture

This document captures the current runtime architecture for `myr`, with emphasis on:

- Event loop and message flow
- Worker lifecycle (connect/query workers + cancellation)
- Action engine invocation path

## Runtime Topology

Primary crates involved in runtime flow:

- `app/src/main.rs`: binary entrypoint, calls `myr_tui::run()`
- `crates/tui/src/lib.rs`: terminal setup/restore + outer render/event loop
- `crates/tui/src/app_logic/*`: message handling, navigation, query/connect orchestration
- `crates/tui/src/lib_helpers.rs`: key mapping and worker functions
- `crates/core/src/actions_engine/*`: action catalog, ranking, enablement, invocation mapping
- `crates/core/src/query_runner.rs`: streaming query loop + cancellation contract

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

## Practical Tracing Checklist

When debugging a runtime behavior:

1. Verify `map_key_event` emits the expected `Msg`.
2. Trace `TuiApp::handle` branch for that `Msg`.
3. For async behavior, follow worker spawn (`start_connect_with_profile` or `start_query_internal`) and the matching `poll_*_result`.
4. For action behavior, inspect `action_context` -> `ActionsEngine::invoke` -> `apply_invocation`.

# Fast MySQL TUI Explorer (Rust) — Detailed Backlog & Tickets (Option A)

**Product:** Fast, interactive MySQL/MariaDB TUI focused on **schema navigation + data exploration**.  
**Core UX:** always show **context-aware “Next actions”** (footer action bar + command palette).

---

## 1) Product goals

**Primary goals**
1. **Fast**: low time-to-connect, low time-to-first-row, smooth scrolling on large results.
2. **Guided**: always offer the best next action based on current context.
3. **Safe by default**: risky SQL requires explicit confirmation.

**Non-goals (v1)**
- IDE-grade SQL autocompletion
- Cross-database support (MySQL/MariaDB only)
- Admin tooling (users/roles/migrations) beyond basic inspection
- Theming ecosystem before performance is locked

**Target users**
- Developers who need to quickly inspect schemas and preview data in terminal
- Users handling large tables who need a client that stays responsive

---

## 2) UX summary (Option A)

**Primary navigation: Schema Explorer**
- Start → Connection Wizard / Profile Select
- Schema view: databases → tables → columns
- Results view: virtualized table, copy/export/search, server-side sort/filter actions

**Secondary**
- Query Editor: helpful for custom queries, but not the “home screen”

**Always visible**
- Header: profile, current DB, latency indicator, SAFE mode, query state
- Footer: “Next actions” (top 5–7 actions), hotkeys **1..7**
- Command palette (Ctrl+P): fuzzy search actions and targets (tables/columns/history)

---

## 3) Technical architecture

### 3.1 State machine + effects (Elm/Redux style)
- **AppState**: all UI + domain state (connection, schema cache, current view, results buffers)
- **Msg**: events (keypresses, timers, async responses, errors)
- **update(AppState, Msg) -> (AppState, Vec<Effect>)**
- **Effect**: async tasks (connect, fetch schema, execute query stream, export) returning Msg

### 3.2 Async model
- Tokio runtime
- DB work always async; UI never blocks
- Ctrl+C cancels running query via cancellation token

### 3.3 Results handling (performance-critical)
- Stream rows; do **not** load all rows
- Store in **bounded ring buffer** (VecDeque)
- Virtual rendering: render only visible rows
- “Preview mode” default suggestion: SELECT without LIMIT triggers suggested action

### 3.4 Suggested crates (keep versions flexible)
- UI: `ratatui`, `crossterm`
- DB: `mysql_async`
- Config: `serde`, `toml`
- Errors: `thiserror`, `anyhow`
- Logging: `tracing` (feature-flag)
- Editor: `tui-textarea` (or equivalent)
- Fuzzy search: `skim` matcher (or equivalent)

---

## 4) Repo structure (recommended)

```
crates/
  core/        # domain: connection, schema, query runner, results buffer, actions engine
  tui/         # ratatui rendering, input mapping, components, layouts
  adapters/    # mysql implementation, exporters, filesystem
app/           # binary entrypoint
docs/          # screenshots/gifs, architecture, contributing
bench/         # docker compose, dataset generator, benchmark scripts
```

---

## 5) Milestones

**M0: Project bootstrap (1–2 days)**
- workspace, CI, license, minimal docs skeleton

**M1: v0.1 “Explorer MVP” (1–2 weeks)**
- connect, schema browse, preview table, results view, next actions bar

**M2: v0.2 “Guided + Palette” (1–2 weeks)**
- polished actions engine, command palette, exports, safe mode confirmations

**M3: v0.3 “Big-table competence” (2–4 weeks)**
- paging strategies, perf overlay, benchmark scripts, regression gates

**M4: v1.0 “Stable OSS release” (time-boxed)**
- hardening, compatibility, packaging, complete docs

---

## 6) Backlog (Epics → Tickets)

### EPIC E00: OSS FOUNDATION
Purpose: repo credibility + contributor readiness.

**E00-001 (P0)** Initialize workspace and crates layout  
- **Scope:** Cargo workspace with crates/core, crates/tui, crates/adapters, app  
- **Acceptance:** `cargo build` works on Linux/macOS; `cargo test` runs  
- **Deps:** none

**E00-002 (P0)** Add license, contributing, code of conduct  
- **Acceptance:** LICENSE (MIT or Apache-2.0), CONTRIBUTING.md, CODE_OF_CONDUCT.md  
- **Deps:** E00-001

**E00-003 (P0)** CI: fmt + clippy + tests + build  
- **Acceptance:** GitHub Actions; `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`  
- **Deps:** E00-001

**E00-004 (P1)** Release pipeline (tag → binaries)  
- **Scope:** cargo-dist or custom packaging workflow  
- **Acceptance:** tag produces binaries for Linux/macOS (Windows optional early)  
- **Deps:** E00-003

**E00-005 (P1)** Docs skeleton  
- **Acceptance:** docs/ with roadmap placeholders  
- **Deps:** E00-002

---

### EPIC E10: CORE DOMAIN
Connections, schema cache, query runner, results buffers.

**E10-001 (P0)** Connection profiles model + config file  
- **Scope:** Profile struct; config load/save to platform path  
- **Acceptance:** create/update/delete profile; persists across runs  
- **Deps:** E00-001

**E10-002 (P0)** Connection manager (async)  
- **Scope:** open/close pool; health check; latency measurement  
- **Acceptance:** connect/disconnect non-blocking; latency in AppState  
- **Deps:** E10-001

**E10-003 (P0)** Schema fetcher + cache  
- **Scope:** list databases, tables, columns; cached with TTL  
- **Acceptance:** schema navigation fast; no refetch on every move  
- **Deps:** E10-002

**E10-004 (P0)** Query runner with streaming rows + cancellation  
- **Scope:** execute SQL with cancellation token; stream into buffer  
- **Acceptance:** Ctrl+C cancels; UI stays responsive while streaming  
- **Deps:** E10-002

**E10-005 (P0)** Results ring buffer + virtual access API  
- **Scope:** bounded VecDeque; cursor model; visible slice API  
- **Acceptance:** memory bounded; smooth navigation in buffer  
- **Deps:** E10-004

**E10-006 (P1)** Safe mode guard for dangerous SQL  
- **Scope:** detect non-read-only statements; require confirmation  
- **Acceptance:** destructive SQL cannot run accidentally  
- **Deps:** E10-004 + confirmation UI

**E10-007 (P1)** SQL generator helpers (explorer actions)  
- **Scope:** preview SELECT, DESCRIBE, SHOW CREATE TABLE, SHOW INDEX, COUNT estimate  
- **Acceptance:** generate correct SQL from selected schema target  
- **Deps:** E10-003

---

### EPIC E20: ACTIONS ENGINE (Context-aware next steps)
**E20-001 (P0)** Action model + registry  
- **Scope:** ActionId enum; Action struct; registry for global + view actions  
- **Acceptance:** actions listable + invokable  
- **Deps:** E10 types

**E20-002 (P0)** Context model  
- **Scope:** track view + selected db/table/col + query + results state  
- **Acceptance:** context updates correctly through navigation/query lifecycle  
- **Deps:** E10-003, TUI view states

**E20-003 (P0)** Action ranker  
- **Scope:** score rules + stable sorting; optional recency boost  
- **Acceptance:** footer shows top N enabled actions; hotkeys mapped  
- **Deps:** E20-001, E20-002

**E20-004 (P1)** Preview-mode suggestion system (LIMIT prompt)  
- **Scope:** detect SELECT w/o LIMIT; suggest “Apply LIMIT 200” action  
- **Acceptance:** suggestion appears; never rewrites silently  
- **Deps:** E20-003, E10-004

**E20-005 (P1)** Palette integration for actions  
- **Scope:** invoke any action from palette  
- **Acceptance:** all actions appear; fuzzy search works  
- **Deps:** E30-006

---

### EPIC E30: TUI APP
Views, components, input mapping.

**E30-001 (P0)** TUI skeleton + render loop  
- **Acceptance:** stable loop; resize handling; no flicker  
- **Deps:** E00-001

**E30-002 (P0)** Global keymap + input handling  
- **Scope:** q quit, ? help, tab panes, ctrl+p palette, ctrl+c cancel, arrows/hjkl  
- **Acceptance:** consistent across views  
- **Deps:** E30-001

**E30-003 (P0)** Connection wizard view  
- **Scope:** form host/port/user/pass/db; test; save profile  
- **Acceptance:** connect and enter schema view  
- **Deps:** E10-001, E10-002

**E30-004 (P0)** Schema explorer view  
- **Scope:** left tree list; right details panel  
- **Acceptance:** instant navigation with cache; selection updates actions  
- **Deps:** E10-003, E20

**E30-005 (P0)** Results view (virtualized table)  
- **Scope:** render visible rows only; cursor selection; copy cell/row  
- **Acceptance:** responsive under streaming; stable cursor  
- **Deps:** E10-005

**E30-006 (P1)** Command palette modal  
- **Scope:** overlay; fuzzy list items; invoke  
- **Acceptance:** ctrl+p opens; enter runs; esc closes  
- **Deps:** E20-005

**E30-007 (P1)** Query editor view (secondary)  
- **Scope:** multiline SQL editor; run/cancel; history  
- **Acceptance:** paste works; run sends to results view  
- **Deps:** E10-004

**E30-008 (P1)** Footer “Next actions” bar  
- **Scope:** show 1..7 actions with hotkeys; disabled state optionally  
- **Acceptance:** always present; updates instantly  
- **Deps:** E20-003

**E30-009 (P1)** Confirmation modal (safe mode)  
- **Scope:** double confirm destructive actions; strong warning UI  
- **Acceptance:** cannot bypass accidentally  
- **Deps:** E10-006

**E30-010 (P2)** Help overlay  
- **Acceptance:** ? opens; documents keys and actions  
- **Deps:** E30-002, E30-008

---

### EPIC E40: EXPLORATION FEATURES (Option A shine)
**E40-001 (P0)** Table preview action (LIMIT N) from schema  
- **Acceptance:** select table → run preview → results view  
- **Deps:** E10-007, E30-004, E30-005

**E40-002 (P1)** Describe table / columns details  
- **Acceptance:** readable display of columns, types, nullability, defaults  
- **Deps:** E10-003

**E40-003 (P1)** Indexes action/view  
- **Acceptance:** SHOW INDEX rendered cleanly  
- **Deps:** E10-003

**E40-004 (P1)** Show Create Table action  
- **Acceptance:** DDL shown in scrollable text viewer + copy  
- **Deps:** E10-007

**E40-005 (P2)** Database switch action  
- **Acceptance:** change active DB without restart; caches refreshed correctly  
- **Deps:** E10-002, E10-003

---

### EPIC E50: EXPORTS & CLIPBOARD
**E50-001 (P1)** Export results to CSV  
- **Acceptance:** writes headers + rows; user chooses path  
- **Deps:** E10-005

**E50-002 (P1)** Export results to JSON  
- **Acceptance:** JSON array of objects col->value  
- **Deps:** E10-005

**E50-003 (P1)** Copy cell/row to clipboard  
- **Acceptance:** works on supported OS; graceful fallback if unsupported  
- **Deps:** E30-005

**E50-004 (P2)** Export schema snapshot (table DDL + indexes)  
- **Acceptance:** single file snapshot per table  
- **Deps:** E40-003, E40-004

---

### EPIC E60: BIG TABLE COMPETENCE (the “fast” differentiator)
**E60-001 (P0)** Perf budget + instrumentation  
- **Scope:** render ms, fps, rows buffered overlay in debug mode  
- **Acceptance:** perf overlay toggleable  
- **Deps:** E30 base

**E60-002 (P1)** Paging: keyset pagination when possible  
- **Scope:** detect PK/unique; paginate with WHERE > last_key ORDER BY  
- **Acceptance:** forward/back pages without OFFSET on PK tables  
- **Deps:** E10-003, E10-007

**E60-003 (P1)** OFFSET fallback paging  
- **Acceptance:** paging works when no suitable key exists  
- **Deps:** E60-002

**E60-004 (P2)** Server-side sort/filter actions  
- **Scope:** sort by selected column; filter prompt builds WHERE clause  
- **Acceptance:** operations remain responsive; avoids huge local transforms  
- **Deps:** prompt input modal + E10-007

**E60-005 (P2)** Search within buffered results (client-side bounded)  
- **Acceptance:** find-next match within buffer quickly  
- **Deps:** E10-005

---

### EPIC E70: RELIABILITY & ERROR UX
**E70-001 (P0)** Unified error model + error panel  
- **Acceptance:** user sees actionable message; details toggle  
- **Deps:** core+tui

**E70-002 (P1)** Reconnect flow  
- **Acceptance:** connection drops prompt reconnect; preserves view when possible  
- **Deps:** E10-002

**E70-003 (P1)** Structured logging feature flag  
- **Acceptance:** tracing logs to file if enabled  
- **Deps:** E00

**E70-004 (P2)** Panic guard + crash report guidance  
- **Acceptance:** friendly panic screen; suggests opening an issue  
- **Deps:** app bootstrap

---

### EPIC E80: SECURITY & CREDENTIALS
**E80-001 (P1)** Password handling strategy  
- **Option A:** prompt each time (default)  
- **Option B:** OS keychain integration (later)  
- **Acceptance:** no plaintext password saved by default  
- **Deps:** E30-003, config

**E80-002 (P1)** TLS modes support  
- **Scope:** disable/require/verify; depends on driver capabilities  
- **Acceptance:** TLS connections possible when configured  
- **Deps:** DB adapter work

**E80-003 (P2)** Improve danger classifier edge cases  
- **Scope:** multi-statement, comments, weird formatting  
- **Acceptance:** fewer false negatives for destructive SQL  
- **Deps:** E10-006

---

### EPIC E90: BENCHMARKS & QUALITY GATES
**E90-001 (P0)** Docker compose + dataset generator  
- **Acceptance:** one command starts DB + loads dataset  
- **Deps:** E00

**E90-002 (P1)** Benchmark script  
Metrics:
- time-to-connect
- time-to-first-row
- rows/sec streaming
- peak memory (best effort)
- **Acceptance:** outputs consistent metrics  
- **Deps:** E90-001, E10-004

**E90-003 (P1)** CI smoke perf checks  
- **Acceptance:** catches major regressions without flakiness  
- **Deps:** E00-003, E90-002

**E90-004 (P2)** Renderer torture test  
- **Acceptance:** no freezes during heavy streaming scenarios  
- **Deps:** E60-001

---

### EPIC E100: DOCUMENTATION & COMMUNITY
**E100-001 (P0)** README with gifs + install  
- **Acceptance:** clear “why”, screenshots, quickstart  
- **Deps:** M1

**E100-002 (P1)** Architecture doc  
- **Acceptance:** explains state machine + effects and how to extend  
- **Deps:** E20/E30

**E100-003 (P1)** “How to add an action” guide  
- **Acceptance:** templates + checklist  
- **Deps:** E20

**E100-004 (P2)** Issue templates  
- **Acceptance:** bug/feature templates for triage  
- **Deps:** E00

---

## 7) Release criteria

**M0 criteria**
- workspace builds, CI passes, license + contributing present

**M1 v0.1 criteria**
- connection wizard + profiles
- schema explorer (db/tables/columns)
- table preview (LIMIT N) → results view
- streaming results + ring buffer + responsive UI
- next actions footer always present and contextual

**M2 v0.2 criteria**
- command palette
- export CSV/JSON
- describe/indexes/show create table actions
- safe mode confirmations
- error UX polished

**M3 v0.3 criteria**
- paging strategies (keyset + fallback)
- perf overlay + bench scripts + regression gates
- basic server-side sort/filter actions

**M4 v1.0 criteria**
- stable cross-platform releases
- safe credentials story (no plaintext by default)
- docs complete, contribution flow validated
- no major perf show-stoppers for big tables

---

## 8) Known risks (explicit)
- Editor complexity: keep minimal early; avoid IDE scope
- Rendering bottlenecks: virtualize aggressively; avoid allocating huge strings
- MySQL/MariaDB differences: fail gracefully; show clear errors
- Cancellation edge cases: design token plumbing from day 1
- Feature creep: “guided actions” stay deterministic until v1+


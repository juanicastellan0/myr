# Roadmap

## M0: Project bootstrap

- [x] Cargo workspace and crate layout
- [x] Baseline CI quality checks
- [x] OSS docs and policies

## M1: Explorer MVP

- [x] Connection profiles and connection wizard (initial implementation)
- [x] Schema explorer (databases, tables, columns)
- [x] Table preview action
- [x] Streaming results with bounded buffer
- [x] Context-aware next actions footer

## M2: Guided exploration

- [x] Command palette (search + keyboard invoke; fuzzy scoring can be improved)
- [x] Safe mode confirmation flow (core guard implemented)
- [x] Export CSV/JSON
- [x] Describe/index/show-create actions

## M3: Big-table competence

- [x] Pagination strategy (keyset + fallback)
- [x] Performance instrumentation overlay
- [x] Benchmark scripts and regression checks

## M4: Stable release

- [x] Packaging hardening
- [x] Cross-platform validation
- [ ] Documentation completion

## M5: Reliability and recovery

- [x] Fix connection/query lifecycle regressions (`Pool was disconnect`) with regression tests
- [x] Auto-reconnect flow with explicit UI state transitions
- [x] Query timeout + retry policy for transient network failures
- [x] Structured error panel with actionable recovery guidance

## M6: Query UX and guided exploration

- [x] Implement real buffered-results search action (replace placeholder)
- [ ] Upgrade query editor usability (multiline edit, movement, history, snippets)
- [ ] Add server-side filter/sort builder from selected schema target
- [ ] Add `EXPLAIN`/preflight action for heavy queries

## M7: Security and safety hardening

- [ ] Optional secure password storage (OS keyring) in addition to env vars
- [ ] Expanded TLS support (CA/cert/key + identity verification options)
- [ ] Explicit read-only profile mode guard
- [ ] SQL audit trail with profile + timestamp metadata

## M8: Power-user workflows and scale

- [ ] Foreign-key relationship navigation (jump across related tables)
- [ ] Saved views/bookmarks for schema targets and queries
- [ ] Extended export options (streaming JSON/CSV improvements, compressed output)
- [ ] Perf trend tracking in CI for regression detection over time

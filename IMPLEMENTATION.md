# IMPLEMENTATION

## Step 1 (Validation Scope): Deterministic Slug Generation

This step converts section **3 (Slugs Spec)** from `FULL_SPEC.md` into a small, testable deliverable.

### Objective
Build and validate a **pure deterministic slug builder** for all configured coins and durations:
- Coins: `BTC`, `ETH`, `SOL`, `XRP`
- Durations: `5m`, `15m`, `1h`, `4h`

### Out of Scope (for this step)
- No network calls (Gamma/CLOB)
- No discovery resolution
- No dashboard wiring
- No engine/runtime orchestration

---

## Functional Requirements

### 1) Public API contract
Implement a single shared slug generation module (no duplicated logic), with a shape equivalent to:

- `build_slug(coin, duration, end_ts_utc, cfg) -> String`

Where:
- `end_ts_utc` is the canonical interval end timestamp in UTC.
- `cfg` includes `PMFLIPS_DISCOVERY_OFFSET_4H_MIN` and timezone handling utilities.

### 2) Templates
- For `5m`, `15m`, `4h`:
  - coin mapping: `BTC->btc`, `ETH->eth`, `SOL->sol`, `XRP->xrp`
  - slug: `{coin_short}-updown-{duration}-{end_ts_s}`
- For `1h`:
  - coin mapping: `BTC->bitcoin`, `ETH->ethereum`, `SOL->solana`, `XRP->xrp`
  - slug: `{coin_full}-up-or-down-{month}-{day}-{hour12}{am_pm}-et`

### 3) Time rules
- `1h` slug date/hour fields must use `America/New_York` wall-clock time (DST-aware).
- `4h` interval alignment must respect `PMFLIPS_DISCOVERY_OFFSET_4H_MIN`.
- No per-coin exceptions.

### 4) Determinism rules
- Same `(coin, duration, end_ts_utc, cfg)` always returns the same slug.
- Function must be side-effect free and independent from I/O.

---

## Validation Plan (must pass before Step 2)

### A. Unit tests for exact known outputs
Create table-driven tests with fixed timestamps and assert exact full slug strings.

Minimum required cases:
1. `5m` basic formatting for all 4 coins.
2. `15m` basic formatting for all 4 coins.
3. `4h` formatting for all 4 coins with non-zero `PMFLIPS_DISCOVERY_OFFSET_4H_MIN`.
4. `1h` formatting for all 4 coins on a normal day (ET).
5. `1h` DST transition coverage:
   - at least one timestamp in EST period
   - at least one timestamp in EDT period

### B. Negative/edge tests
- Invalid duration -> explicit error.
- Unsupported coin -> explicit error.
- Boundary times exactly on interval end should format correctly.

### C. Property-style checks (small deterministic sweep)
For a bounded timestamp range:
- Repeated calls return identical outputs.
- Output charset and pattern match the relevant template per duration.

---

## Acceptance Criteria
Step 1 is complete only when all are true:
1. Slug builder exists as a single shared implementation.
2. `5m/15m/1h/4h` rules are implemented exactly per spec.
3. `1h` uses ET wall-clock conversion with DST handling.
4. `4h` uses `PMFLIPS_DISCOVERY_OFFSET_4H_MIN`-based alignment.
5. All validation tests pass in CI/local.
6. No network dependencies are required for Step 1 tests.

---

## Deliverables
- Slug generation module and public interface.
- Unit + edge + deterministic sweep tests.
- Short README note (or module docs) describing template and timezone behavior.

---

## Step 2 (Validation Scope): Discovery Resolution (SDK-first)

This step converts section **6.1 / Step 2** from `FULL_SPEC.md` into a contained deliverable.

### Objective
Build and validate a discovery-resolution layer that:
- resolves deterministic slugs through official Polymarket SDK metadata paths
- keeps unresolved slugs visible as typed placeholders

### Functional Requirements
1. Add typed discovery entities:
- `DiscoveryKey { coin, duration, end_ts_utc, slug }`
- `DiscoveryRow<M> { key, status }`
- `DiscoveryStatus<M> = Resolved { market } | Unresolved { reason }`
- `UnresolvedReason = NotFound | TransportError(String)`

2. Implement batch resolver behavior:
- preserve input order
- deduplicate slugs before fetch and re-expand afterwards
- never drop unresolved rows

3. Add SDK-backed live path:
- `resolve_discovery_batch(keys, cfg)` under feature-gated SDK integration
- retry + timeout behavior from explicit config

### Validation Plan (must pass before Step 3)
1. Unit tests:
- order preserved after dedupe/re-expand
- duplicate slugs fetched once
- missing slug -> `Unresolved(NotFound)`
- transport error -> `Unresolved(TransportError)`
- output length equals input length

2. Live integration test:
- run against Gamma under feature flag
- verify at least one resolved row and one guaranteed invalid unresolved row

### Acceptance Criteria
Step 2 is complete only when all are true:
1. Discovery module resolves slug batches via official SDK path.
2. Unresolved slugs are explicit typed rows.
3. No unresolved row suppression is possible via resolver output.
4. Unit tests for mapping logic pass.
5. Live Gamma integration test is available and feature-gated.

---

## Step 3 (Validation Scope): Dashboard Core Table

This step converts section **6.1 / Step 3** and dashboard requirements from section **2** in `FULL_SPEC.md` into a contained deliverable.

### Objective
Build and validate a browser-visible dashboard core table that:
1. Renders exactly one page with exactly one large table (no ladder view).
2. Shows all required columns from `FULL_SPEC.md` section 2.5.
3. Includes a clickable market `Link` derived from slug.
4. Keeps unresolved markets visible in table rows (no silent suppression).

### Out of Scope (Step 3 only)
1. Checkbox filters (`coin/duration/bets_open/in_interval`) and filter logic.
2. Formatting polish rules (significant digits, percent format, etc.).
3. `mock_columns` visual highlighting behavior.
4. Shared-memory IPC integration (use pluggable source, mock default now).
5. Trading/evaluation logic and any execution-path coupling.

### Functional Requirements
1. **UI/Server shape**
- Rust HTTP server with SSR HTML page at `/dashboard`.
- Single table component only.
- Optional JSON snapshot endpoint for diagnostics (read-only).

2. **Snapshot source abstraction**
- Define a dashboard snapshot reader interface (trait) with read-only fetch:
  - `snapshot() -> DashboardSnapshot`
- Provide in-memory mock implementation for Step 3 manual validation.
- Keep interface ready to swap with shared-memory provider later.

3. **Row model and required columns**
- Table rows must render these 22 columns in order:
  1. `Link`
  2. `Coin`
  3. `Duration`
  4. `Bets Open`
  5. `In Interval`
  6. `End` (`hh:mm`)
  7. `Midprice`
  8. `Best Bid YES`
  9. `Best Ask YES`
  10. `Position Net` (`size@price@YES|NO`)
  11. `Pos YES` (`size@price`)
  12. `Pos NO` (`size@price`)
  13. `Offer YES` (`size@price`)
  14. `Offer NO` (`size@price`)
  15. `Net Profit`
  16. `Fee %`
  17. `Reward %`
  18. `P_finished`
  19. `P_running`
  20. `P_next`
  21. `dist1(mu,sigma,nu,lambda)`
  22. `dist2(mu,sigma,nu,lambda)`

4. **Link behavior**
- `Link` uses slug-based URL: `https://polymarket.com/event/{slug}`.
- Link text should be human-usable (slug or compact label), clickable in browser.

5. **Missing/unresolved behavior**
- If market is unresolved or field unavailable, row still renders.
- Unavailable cells use placeholder (`-`) for this step.
- No row dropping due to missing metadata.

6. **Control-plane isolation**
- Dashboard is read-only and isolated from execution path.
- Failures in dashboard render/path must not mutate discovery state.

### Important API / Interface Additions
1. `DashboardSnapshot` (table-level payload for current render cycle).
2. `DashboardRow` (one row with all required table fields, allowing missing values).
3. `DashboardSnapshotSource` trait (read-only data access abstraction).
4. HTTP route contracts:
- `GET /dashboard` -> SSR HTML page with single table.
- Optional `GET /dashboard/snapshot` -> JSON snapshot for debugging/manual checks.

### Validation Plan (must pass before Step 4)
1. **Unit tests**
- Header order and exact column count (22) are correct.
- Row-to-cell mapping fills all columns.
- Unresolved row remains visible and uses placeholders.
- Link generation from slug matches expected URL format.

2. **Integration tests**
- `GET /dashboard` returns HTTP 200 and includes table + required headers.
- Rendering with mixed resolved/unresolved rows still returns full row count.
- Mock snapshot source wiring works end-to-end.

3. **Manual browser checks**
- Open dashboard in browser.
- Confirm one page, one table, no ladder view.
- Confirm all required columns appear.
- Confirm link click opens expected Polymarket event URL pattern.

### Acceptance Criteria
Step 3 is complete only when all are true:
1. Dashboard serves a single-page, single-table view.
2. All 22 required columns are present in required order.
3. Market `Link` column is clickable and slug-derived.
4. Unresolved rows remain visible (not suppressed).
5. Automated unit/integration checks pass.
6. Manual browser validation confirms table renders correctly.

### Deliverables
1. Step 3 section added to `IMPLEMENTATION.md` with the above scope/checks.
2. Dashboard core-table implementation plan locked to Rust SSR + pluggable snapshot source.
3. Explicit test checklist for automated + manual validation.
4. Clear boundary between Step 3 and Step 4 responsibilities.

### Assumptions and Defaults
1. Use mock in-memory snapshot provider for Step 3 unless shared-memory reader already exists.
2. Default unresolved cell placeholder is `-` in Step 3.
3. Link source is slug (not token-id URL) for this step.
4. Dashboard refresh/perf tuning beyond simple render correctness is deferred until later dashboard logic/optimization steps.

---

## Step 6 (Validation Scope): Global Logging Baseline (Control-Plane First)

This step skips Step 5 (contract freeze) and adds a shared structured logging baseline across `dashboard_server`, `dashboard`, `discovery`, and `slug`-adjacent control-plane flows.

### Objective
Implement logging that is:
1. Structured and machine-readable.
2. Consistent across modules.
3. Useful for end-to-end control-plane debugging (`startup -> discovery cycle -> snapshot render`).
4. Low-noise by default, with higher-detail opt-in.

### Out of Scope
1. Execution-path logging (engine/send path).
2. Metrics backend (`prometheus`/OTel exporter).
3. Contract freeze/schema gate.
4. Distributed tracing across processes.

### Functional Requirements
1. Add shared observability module:
- `src/observability.rs`
- `LoggingConfig { level, format, include_target }`
- `LogFormat = Json | Pretty`
- `init_logging(&LoggingConfig) -> Result<(), LoggingInitError>`
- `logging_config_from_env() -> LoggingConfig`

2. Environment contract:
- `PMM_LOG_LEVEL` (default `info`)
- `PMM_LOG_FORMAT` (`pretty|json`, default `pretty`)
- `PMM_LOG_TARGET` (`true|false`, default `true`)

3. Stable event naming baseline:
- `app.start`
- `app.bind`
- `source.selected`
- `discovery.cycle.start`
- `discovery.cycle.finish`
- `discovery.resolve.error`
- `discovery.degraded.row_transport`
- `discovery.degraded.batch_transport`
- `http.dashboard.request`
- `http.snapshot.request`

4. Logging behavior:
- Replace runtime `println!/eprintln!` with `tracing` macros.
- Include context fields for cycle/request/degraded summaries.
- `info`: lifecycle + cycle summaries + high-signal errors.
- `debug`: row-level transport degraded events.

### Validation Plan (must pass before next step)
1. Unit tests:
- parse `PMM_LOG_FORMAT` and fallback behavior.
- parse `PMM_LOG_LEVEL` and defaulting behavior.
- `logging_config_from_env` deterministic behavior.

2. Logging smoke tests:
- startup emits `app.start`, `source.selected`, `app.bind`.
- snapshot request emits `http.snapshot.request`.
- simulated discovery transport error emits `discovery.resolve.error` and `discovery.degraded.batch_transport`.
- row transport degraded emits `discovery.degraded.row_transport` at debug level.

3. Manual checks:
- run with `PMM_LOG_FORMAT=json PMM_LOG_LEVEL=info`.
- verify cycle summary visibility without row-level spam.
- verify degraded scenarios are visible and explicit.

### Acceptance Criteria
Step 6 is complete only when all are true:
1. Shared observability setup is used by all current control-plane modules.
2. No operational `println!/eprintln!` remain in runtime paths.
3. Lifecycle/error/degraded events are visible at info/error levels.
4. Debug level adds row-level degraded visibility without changing info noise profile.
5. Logging smoke tests pass and README documents env usage.

### Deliverables
1. Step 6 section in `IMPLEMENTATION.md`.
2. New observability module with env-configurable logging initialization.
3. Structured logging in server + dashboard + discovery control-plane paths.
4. Automated logging smoke tests and documentation updates.

### Assumptions and Defaults
1. Step 5 is skipped; log field sets are best-effort stable (not schema-frozen yet).
2. Default local format is `pretty`; CI/reliability runs can use `json`.
3. Discovery refresh cadence remains current, and logging must stay lightweight.
4. Event names above are baseline vocabulary for future steps.

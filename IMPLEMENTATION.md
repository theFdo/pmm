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

---

## Step 8 (Validation Scope): Historical Klines Loader (Downloader Included)

This step skips Step 7 (by explicit decision) and adds historical Binance `1s` kline ingestion for offline/runtime cold-start inputs.

### Objective
Implement a production-ready historical `1s` loader for:
1. `BTCUSDT`
2. `ETHUSDT`
3. `SOLUSDT`
4. `XRPUSDT`

with deterministic archive planning, downloader/cache behavior, archive parsing, and explicit coverage reporting.

### Out of Scope
1. Live websocket ingestion/reconnect.
2. Recent-hours REST synchronization.
3. Bars-to-features transform (Step 9).
4. Model training/inference wiring.

### Functional Requirements
1. Deterministic archive planning:
- monthly path: `.../monthly/klines/{SYMBOL}/1s/{SYMBOL}-1s-YYYY-MM.zip`
- daily path: `.../daily/klines/{SYMBOL}/1s/{SYMBOL}-1s-YYYY-MM-DD.zip`
- full months use monthly archives
- partial boundary months use daily archives

2. Downloader behavior:
- write to local cache under `data_root/{SYMBOL}/1s/{monthly|daily}/`
- atomic writes (`.tmp` then rename)
- retry with exponential backoff
- optional checksum verification (`.CHECKSUM`)
- checksum-valid cached files are reused

3. Parser behavior:
- parse first CSV entry from ZIP archive
- enforce Binance kline schema and numeric parsing
- keep only rows in requested `[start, end)` interval
- sort ascending by `open_time_ms`
- dedupe by `open_time_ms` (first kept deterministically)

4. Coverage/gap behavior:
- canonical step size: `1000ms`
- include explicit report fields:
  - `expected_points`
  - `actual_points`
  - `missing_points`
  - `duplicate_points_removed`
  - `gap_ranges` (bounded)
  - `total_gap_ranges`
- no gap imputation; loader succeeds with `allow + report`

5. Observability:
- emit structured events:
  - `binance.sync.start`
  - `binance.sync.file.cached`
  - `binance.sync.file.downloaded`
  - `binance.sync.file.checksum_failed`
  - `binance.load.finish`
  - `binance.load.gap_detected`

### Public API Additions
1. `BinanceSymbol`
2. `Kline1s`
3. `KlineLoadRequest`
4. `KlineCoverageReport`
5. `KlineLoadResult`
6. `HistoricalKlinesConfig`
7. `plan_required_archives(req) -> Vec<ArchiveRef>`
8. `sync_archives(req, cfg) -> Result<Vec<LocalArchive>, KlineLoadError>`
9. `load_1s_klines(req, cfg) -> Result<KlineLoadResult, KlineLoadError>`

### Validation Plan (must pass before Step 9)
1. Unit tests:
- archive planning across month boundaries
- kline CSV schema/field parsing
- duplicate + gap coverage calculations
- checksum mismatch rejection when verification is enabled

2. Integration tests:
- fixture load test per symbol (`BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `XRPUSDT`)
- monthly+daily stitch behavior with overlap dedupe
- coverage report correctness for missing timestamps
- cache-hit sync behavior without network

3. Optional live smoke:
- feature-gated network test for one known Binance archive
- ignored by default

### Acceptance Criteria
Step 8 is complete only when all are true:
1. Loader supports all four required symbols.
2. Monthly/daily planning is deterministic.
3. Downloader cache + checksum behavior is implemented.
4. Parsed outputs are filtered, sorted, deduped.
5. Missing timestamps are visible via explicit coverage report.
6. Fixture/integration tests pass without external network.
7. README and implementation docs describe Step 8 behavior.

### Deliverables
1. `src/binance_klines.rs` module.
2. `lib.rs` exports for Step 8 public API.
3. Fixture and integration tests for loader behavior.
4. Documentation updates in `README.md` and this file.

### Assumptions and Defaults
1. Step 7 remains skipped intentionally.
2. Step 8 includes downloader implementation now.
3. Gap policy: `allow + report`.
4. Time axis is UTC milliseconds.
5. Only `1s` interval archives are in scope.

---

## Step 9 (Validation Scope): Shared Bars-to-Features Transform

This step adds one shared deterministic bars-to-features transform for both training prep and runtime cold-start paths.

### Objective
Build and validate a shared transform that:
1. Reads aligned `1s` klines for `BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `XRPUSDT` from the Step 8 combined SQLite store.
2. Produces deterministic feature rows at `1s` cadence.
3. Exposes explicit schema version + fingerprint for compatibility checks.
4. Reuses one transform code path for training/runtime wrappers.

### Out of Scope
1. Model training/artifact production (Step 10).
2. Websocket transport/reducer implementation (Steps 11-12).
3. Readiness gates and execution wiring (Step 13+).
4. Trading/action logic coupling.

### Functional Requirements
1. Shared module + API:
- `src/features.rs`
- no duplicate transform math in binaries/tests

2. Input contract:
- read from `klines_1s` table in combined store
- process range `[start_ts_ms_utc, end_ts_ms_utc_exclusive)`

3. Determinism:
- stable row ordering by timestamp ascending
- stable column ordering from schema builder
- same input + config => same schema fingerprint and output rows

4. Feature families (`f64`, v1):
- per symbol:
  - `ret_1s = ln(close_t / close_{t-1})`
  - `ret_w = ln(close_t / close_{t-w})`
  - `range_w = ln(max_high_w / min_low_w)`
  - `vol_w = stddev(ret_1s over w)`
  - `quote_vol_w = ln(1 + sum(quote_asset_volume_w))`
- global:
  - `tow_sin`, `tow_cos`
- shared helper:
  - `horizon_conditioning(horizon_seconds, max_duration_seconds)`

5. Gap behavior:
- enforce 1-second monotonic continuity and all-4-symbol completeness
- `GapPolicy::Strict`: fail on first continuity/incomplete-frame issue
- `GapPolicy::ReportAndSkip`: continue, reset rolling window segment, and report gaps

6. Schema/version compatibility:
- explicit `FEATURE_SCHEMA_VERSION`
- deterministic schema fingerprint from ordered columns + transform params
- compatibility assertion helper for future training/runtime gates

7. Observability:
- `features.schema.built`
- `features.transform.start`
- `features.transform.gap_detected`
- `features.transform.finish`

### Public API Additions
1. `FeatureTransformConfig`
2. `FeatureTransformRequest`
3. `GapPolicy`
4. `FeatureDType`
5. `FeatureColumn`
6. `FeatureSchema`
7. `FeatureRow`
8. `FeatureTransformReport`
9. `HorizonConditioning`
10. `FeatureError`
11. `FEATURE_SCHEMA_VERSION`
12. `build_feature_schema(cfg) -> FeatureSchema`
13. `transform_store_range(store_path, req, cfg) -> Result<(FeatureSchema, Vec<FeatureRow>, FeatureTransformReport), FeatureError>`
14. `transform_store_range_for_training(...) -> ...`
15. `transform_store_range_for_runtime_cold_start(...) -> ...`
16. `horizon_conditioning(...) -> HorizonConditioning`
17. `assert_schema_compatible(expected_version, expected_fingerprint, actual_schema) -> Result<(), FeatureError>`

### Validation Plan (must pass before Step 10)
1. Unit tests:
- schema order and deterministic fingerprint
- `horizon_conditioning` numeric correctness
- known-value math checks for feature families
- warm-up behavior with windowed features

2. Integration tests:
- temp SQLite store transform with all 4 symbols aligned
- deterministic repeat run equality
- strict-mode incomplete frame failure
- strict-mode continuity gap failure
- report-and-skip mode returns rows + explicit gap report

3. Contract-style tests:
- schema compatibility pass/fail on version/fingerprint mismatch
- training/runtime wrappers return identical output for same input/config

### Acceptance Criteria
Step 9 is complete only when all are true:
1. One shared transform implementation powers both wrappers.
2. Schema version + fingerprint are deterministic and test-covered.
3. Gaps are explicit and policy-controlled (strict vs report-and-skip).
4. Time-of-week encoding and horizon-conditioning helpers are implemented.
5. Unit + integration tests pass.
6. README and implementation docs are updated.

### Deliverables
1. `src/features.rs` shared transform module.
2. `src/lib.rs` Step 9 exports.
3. `tests/features_transform.rs` deterministic/gap/compatibility coverage.
4. Documentation updates in `README.md` and this file.

### Assumptions and Defaults
1. Step 7 remains skipped by explicit decision.
2. Input cadence is `1s` UTC milliseconds.
3. Default gap policy is `Strict`.
4. Default max duration is `86400` seconds.
5. V1 feature dtype is `f64`.

# MODULAR SPEC (Functional + Minimal, Decision-Oriented)

## 1) Goal
Define a simpler architecture where each part is independently implementable and replaceable.

System scope:
1. Discover Polymarket markets (`previous`, `active`, `next`) per coin/duration.
2. Maintain complete Binance 1s price history + near-now sync.
3. Build deterministic features from bars.
4. Run a replaceable Dist module.
5. Derive probabilities and actions from a small runtime state.
6. Expose read-only dashboard visibility.

---

## 2) Global Rules
1. Functional core, adapter shell:
- Pure functions for scheduling, transforms, probability math.
- IO modules only for HTTP/WS/DB/process orchestration.

2. Explicit degraded states:
- Missing data becomes typed degraded output, never silent drop.

3. Single writer per state:
- Each module has one reducer/writer; consumers read snapshots.

4. Deterministic contracts:
- Same input + config => same output, including schema/fingerprint.

5. Control plane isolation:
- Dashboard/diagnostics must not block probability/action path.

---

## 3) Modules (Concrete)

## M1. Market Catalog (Slug + Discovery)
Purpose:
- Compute expected market windows and slugs.
- Resolve metadata through SDK.

Inputs:
- `now_utc`
- universe: `coins x durations`
- slug rules (ET-aligned scheduling)

Outputs:
- `Vec<MarketRow>`
- one row per `(coin,duration,window in {previous,active,next})`

Contract:
- output length is always fixed (`coins * durations * 3`)
- rows are never dropped; unresolved rows are explicit

Core API:
- `schedule_market_keys(now_utc) -> Vec<MarketKey>`
- `build_slug(key) -> String`
- `resolve_markets(keys) -> Vec<DiscoveryRow>`

Cadence:
- 250ms to 1s refresh acceptable (control-plane loop).

---

## M2. Price Store (Historical + Fill + Tail)
Purpose:
- Maintain one combined 1s kline store for all symbols.

Inputs:
- Binance monthly/daily archives
- Binance REST tail sync
- later: WS continuous append

Outputs:
- durable `klines_1s` store
- coverage reports (missing/duplicate ranges)

Contract:
- data keyed by `(symbol_id, open_time_ms)`
- query range semantics: `[start_ms, end_ms)`
- no imputation in store layer

Core API:
- `sync_to_now(symbols, start_ms, now_ms) -> SyncReport`
- `query_aligned_frames(symbols, start_ms, end_ms) -> Vec<KlineFrame>`
- `assert_complete(symbols, start_ms, end_ms) -> Result<()>`

Cadence:
- startup catch-up, then periodic tail sync.

---

## M3. Feature Engine (Shared Transform)
Purpose:
- Convert aligned 1s bars into deterministic feature rows for both training and runtime cold-start.

Inputs:
- aligned frames from M2
- feature config (`windows`, `max_duration_seconds`, gap policy)

Outputs:
- `FeatureSchema { version, fingerprint, columns }`
- `Vec<FeatureRow>`
- `FeatureTransformReport`

Contract:
- same transform function for training + runtime cold-start
- stable column ordering + fingerprint
- gap policy:
  - `Strict` -> fail fast
  - `ReportAndSkip` -> continue with explicit report

Core API:
- `build_feature_schema(cfg) -> FeatureSchema`
- `transform_frames(frames, cfg) -> (schema, rows, report)`
- `assert_schema_compatible(expected, actual) -> Result<()>`

Cadence:
- batch/offline for training; startup batch for runtime warmup.

---

## M4. Dist Module (Replaceable)
Purpose:
- Produce distribution outputs and own dist semantics.
- Define probability-derivation interface from dist output.
- Be swappable without changing upstream modules.

Inputs:
- latest valid feature sequence
- market timing context (`now`, `start`, `end`, `duration`)
- current market price context (`price_now`, `ref_price`)

Outputs:
- `DistOutput` containing two dist entries per market context:
  1. `dist_h1` with `h1 = max(end_ts - now_ts, 0)` (time left)
  2. `dist_h2` with `h2 = h1 + duration_seconds` (time left + duration)

Important requirement:
- For each `(coin,duration,window row)`, both dists are produced every inference cycle.
- This is mandatory because non-active rows still need a probability path.

Contract:
- module defines:
  - output parameter size/type (for example tuple length)
  - deterministic serialization shape
  - probability derivation adapter signature
- all-or-nothing batch commit: partial invalid dist set is rejected

Core API:
- `infer_dist(feature_seq, horizons) -> DistOutput`
- `derive_probability(dist, ref_price, price_now, market_state) -> f64`
- `validate_dist_output(output) -> Result<()>`

Notes:
- `ref_price` (price at interval start) is required probability input.
- Dist module owns dist-to-probability math so it can be replaced later.

Cadence:
- fixed 1s inference cycle.

---

## M5. Probability + Action Core
Purpose:
- Apply trigger logic and produce action decisions from latest state.

Inputs:
- market catalog snapshot (M1)
- current price/ref_price state
- dist output + probability adapter (M4)
- PM book/position state

Outputs:
- `ProbabilitySnapshot` for all visible rows
- `ActionDecision | NoAction`

Probability policy (minimal):
1. Active interval (`start <= now < end`):
- primary probability uses `dist_h1` and `ref_price`.

2. Not yet in interval (`now < start`):
- probability uses `dist_h2` and `ref_price`.
- this keeps "next" rows probabilistically defined.

3. Finished interval (`now >= end`):
- use resolved outcome if available; otherwise degraded placeholder.

Trigger policy:
- recompute on any:
  - price update
  - dist update
  - PM book update

Core API:
- `recompute_probabilities(state) -> ProbabilitySnapshot`
- `evaluate_action(state) -> ActionDecision`

---

## M6. Dashboard (Read-Only Projection)
Purpose:
- Render operator table and filters from snapshot state.

Inputs:
- read-only combined snapshot (M1 + M5 + PM fields)

Outputs:
- `/dashboard` HTML
- `/dashboard/snapshot` JSON

Contract:
- unresolved/degraded rows stay visible
- mock/degraded fields explicitly marked
- no write path to runtime state

Core API:
- `build_dashboard_snapshot(state) -> DashboardSnapshot`
- `render_dashboard(snapshot, filters) -> Html`

Cadence:
- client polling 100-250ms.

---

## M7. Runtime Orchestrator
Purpose:
- Wire loops, reducers, and lifecycle without business logic duplication.

Responsibilities:
1. Startup sequence:
- logging init
- M2 catch-up + completeness check
- M1 discovery start
- M3 warmup + schema check
- M4 inference loop start
- M6 server start

2. Event routing:
- IO adapters publish events
- reducers update module-local state
- M5 recompute/eval triggered by event types

3. Safety gates:
- external send disabled until readiness conditions pass

---

## 4) Shared Contracts (Minimal Types)
1. `MarketKey { coin, duration, window, start_ts, end_ts, slug }`
2. `MarketRow { key, status }`
3. `PriceContext { ts_ms, price_now, ref_price }`
4. `KlineFrame { ts_ms, per_symbol_ohlcv }`
5. `FeatureRow { ts_ms, values, schema_fingerprint }`
6. `DistEntry { horizon_s, params }`
7. `DistOutputRow { key, dist_h1, dist_h2 }`
8. `ProbabilityRow { key, probability, source, degraded_reason? }`
9. `RuntimeSnapshot { markets, prices, probs, pm_fields, mock_columns }`

---

## 5) Minimal Implementation Path
1. M1 + M2 stable and deterministic.
2. M3 deterministic transform + schema compatibility checks.
3. M4 dist interface + dual-horizon output + `ref_price`-aware probability adapter.
4. M5 trigger wiring and action decisions.
5. M6 dashboard projection.
6. M7 lifecycle/readiness/send-gate orchestration.

Each step must ship:
1. API contract tests.
2. Deterministic behavior tests.
3. Degraded-path tests.

---

## 6) Suggested File Layout
- `src/market_catalog.rs` (M1)
- `src/price_store.rs` (M2)
- `src/features.rs` (M3)
- `src/dist.rs` (M4)
- `src/prob_action.rs` (M5)
- `src/dashboard.rs` (M6)
- `src/runtime.rs` (M7)
- `src/contracts.rs` (shared minimal types)

---

## 7) Non-Goals
1. Detailed model architecture/hyperparameters.
2. UI styling details.
3. Exchange-specific send strategy details.

This spec is intentionally modular and short so each part can be built independently.

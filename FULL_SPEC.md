# Full Specification

This document is the consolidated implementation reference for the system.
It combines high-level system behavior, dashboard requirements, slug generation rules,
execution/probability paths, model requirements, and implementation/testing order.

## 1) System High-Level Spec

### 1.1 Goal
Build a production-grade, low-latency trading system with strong determinism, clear validation gates,
and operator visibility that never interferes with critical execution paths.

### 1.2 Scope
- Language: Rust.
- Polymarket integration must use the official Polymarket Rust SDK (`rs-clob-client` / `polymarket-client-sdk`), not custom ad-hoc API clients.
- Coins: `BTC`, `ETH`, `SOL`, `XRP`.
- Binance pairs: coin against `USDT` (`BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `XRPUSDT`).
- Durations: `5m`, `15m`, `1h`, `4h`.
- Market family: Polymarket crypto up/down markets.
- Data inputs: Binance (price/bars), Polymarket Gamma (discovery/metadata), Polymarket CLOB (book and later execution-facing market data).

Binance input sources are mandatory:
- Live high-frequency input: Binance orderbook websocket channels, used for both:
  - high-frequency midprice updates, and
  - continuous construction of sealed 1s klines to keep model-input sequences up to date.
- Historical bar files for training and cold-start cache: Binance data portal monthly and daily 1s klines (e.g. `https://data.binance.vision/?prefix=data/spot/monthly/klines/BTCUSDT/1s/` and corresponding daily path).
- Recent past-hours synchronization: Binance API.

### 1.3 Planes
- Execution plane: `price/dist/prob/action/order`.
- Control plane: discovery, metadata refresh, dashboard, health, diagnostics.

Hard rule:
- Control-plane work must not block execution-plane paths.

### 1.4 Core runtime principles
- Deterministic behavior where feasible.
- Single-writer state reducers; readers consume snapshots.
- Explicit mock-vs-live visibility.
- Fail degraded, not invisible: missing upstream data should not silently drop required visibility.
- Minimize custom data-structure definitions: prefer reusing existing SDK/library types directly instead of creating local subset structs.
- Logic duplication is not allowed; shared logic must be implemented once and reused.
- Probability derivation logic must be implemented once and reused everywhere.
- Use official SDK/library types and clients wherever possible (especially Polymarket SDK paths); avoid local redefinitions when SDK coverage exists.

## 2) Dashboard Spec

### 2.1 Purpose
Operator view for validation and monitoring, not decision logic.

### 2.2 Data source
- Shared-memory snapshot only.
- Dashboard process is read-only.
- Target refresh cadence: `100-250ms`.

### 2.3 Layout
- Single page.
- One large table only.
- No ladder view.

### 2.4 Filters (checkbox lists)
- `Coin`
- `Duration`
- `Bets Open`: `open`, `closed`
- `In Interval`: `yes`, `no`

Filter logic:
- OR within a filter group.
- AND across groups.
- Default: all checked.

### 2.5 Required columns
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

### 2.6 Formatting rules
- Prices/sizes: max 4 significant digits.
- Dist params/fees/rewards: max 3 significant digits.
- Probabilities: percent display.
- Unavailable value: `-`.
- Missing fee/reward: `0`.

### 2.7 Mock marking
- Any mock-backed field must be visibly marked.
- `mock_columns` mapping is mandatory and must stay in sync with UI fields.

### 2.8 Field clarifications
- `Bets Open`: means the market is currently bettable (`open`/`closed`), sourced from API metadata.
- `In Interval`: `yes` only when `interval_start <= now < interval_end` for the row's active interval context; otherwise `no`.
- `Best Bid YES` / `Best Ask YES`: YES side only.
- `Position Net`: exactly one side (`YES` or `NO`) shown as `size@price@SIDE`.
- `Pos YES` / `Pos NO` and `Offer YES` / `Offer NO` are condensed `size@price` fields.
- `Net Profit` is mark-to-book from current position and visible book-side prices.

## 3) Slugs Spec

### 3.1 Deterministic generation
Slugs are generated deterministically from `(coin, duration, end timestamp)` regardless of network status.

### 3.2 Templates
For `5m`, `15m`, `4h`:
- `BTC -> btc`, `ETH -> eth`, `SOL -> sol`, `XRP -> xrp`
- Template: `{coin_short}-updown-{duration}-{end_ts_s}`
- Examples:
  - `btc-updown-5m-1771449000`
  - `eth-updown-15m-1771448400`
  - `xrp-updown-4h-1771448400`

For `1h`:
- `BTC -> bitcoin`, `ETH -> ethereum`, `SOL -> solana`, `XRP -> xrp`
- Template: `{coin_full}-up-or-down-{month}-{day}-{hour12}{am_pm}-et`
- Examples:
  - `bitcoin-up-or-down-february-18-4pm-et`
  - `ethereum-up-or-down-february-18-5pm-et`

1h time interpretation rule:
- Use `America/New_York` local wall-clock time (DST-aware) when constructing the `1h` slug date/hour fields.
- Do not use a configurable global offset for `1h` slug formatting.

### 3.3 4h alignment
- Must use offset config `PMFLIPS_DISCOVERY_OFFSET_4H_MIN`.
- No per-coin hard-coded exceptions.

### 3.4 Resolution behavior
- Resolve slugs in batch via Gamma SDK.
- Unresolved slugs must remain visible as placeholders; never silently suppress them.

## 4) Paths and Probabilities Spec

### 4.1 Mandatory execution triggers
- Price path:
  - `binance midprice change` -> recompute probs -> action eval trigger.
- Dist path:
  - atomic dist batch update (all coin/duration together, 1s cadence)
  - -> recompute probs -> action eval trigger.
- PM path:
  - PM top/book change -> action eval trigger.

Any of those events must trigger evaluation.

Execution ordering requirement:
- This rule applies to all jumps in all execution paths defined by this spec.
- Every jump must run sequentially with no intentional delay stage between steps, i.e. next-step code flow.

### 4.2 Dist semantics
- Model inference must run every 1 second.
- Each 1-second inference cycle produces the dist update candidate for that cycle.
- Dist batch is all-or-nothing.
- Partial invalid batch must be rejected.

### 4.3 Probability semantics
Per active context:
- `dist1`: horizon = remaining active-market time.
- `dist2`: horizon = `dist1 + duration`.
- `ret_since_active_start`: required input.

Derived:
- `P_finished`: authoritative resolved source only when available.
- `P_running`: from `dist1` + return context.
- `P_next`: deterministic quick method acceptable for now.

### 4.4 Isolation
Execution path must not block on dashboard rendering, specs/docs processing, or metadata refresh.

## 5) Model Spec

### 5.1 Objective
Predict per-coin distribution parameters (currently zero-mean skew Student-t) used by runtime probability estimation.

### 5.2 Inputs
- Sequence input over base cadence `1s` (current default).
- Features for all coins, across configurable time windows.
- Configurable:
  - sequence length
  - feature set
  - time windows

Per-window periodic time encoding (configurable):
- periodic features (current: `tow`) are encoded using:
  - `sin(periodic_feature)`
  - `cos(periodic_feature)`

Global horizon conditioning sequences:
- `log_horizon`
- `sqrt_horizon`
- normalized by max duration.

### 5.3 Architecture
- LSTM backbone + configurable head.
- Layers, widths, norms, and related parameters are configurable.

### 5.4 Training objective
- NLL on log returns from `now` to `now+horizon_seconds`.

Sampling from historical crypto data:
1. Pick timestep.
2. Pick random duration from available durations.
3. Pick random forecast horizon where `horizon < duration`.
4. Build sample and target.

### 5.5 Validation against historical PM trades
Compute BCE for:
- Trades: probability from trade price.
- Model: probability from output distribution using `-returns_since_market_started`.

### 5.6 Shared probability math requirement
Probability-derivation functions for model training/validation must use the same runtime implementation (or bit-for-bit equivalent wrapper).
No divergent math paths.

## 6) Implementation Spec

### 6.1 Step-by-step plan (small contained steps)
1. Slug Generation
- Implement: deterministic slug builder for `5m/15m/1h/4h` (including 1h NY DST rule and 4h offset).
- Check: unit tests with exact expected slugs.

2. Discovery Resolution (SDK-first)
- Implement: batch slug resolution via Polymarket Rust SDK, using SDK metadata types as source objects.
- Check: integration test for found and unresolved slugs; unresolved rows remain visible.

3. Dashboard Core Table
- Implement: one-table dashboard with required columns and market link.
- Check: manual browser validation that all columns render.

4. Dashboard Logic Pack
- Implement: checkbox filters (`coin/duration/bets_open/in_interval`), `in_interval` rule (`start <= now < end`), formatting rules.
- Check: UI logic tests + manual checks on filter behavior and boundary times.

5. Contract Freeze Checkpoint
- Implement: freeze current event names, required fields, and `mock_columns` semantics used by discovery/dashboard.
- Check: contract tests and schema compatibility checks pass before moving forward.

6. Global Logging Baseline
- Implement: cross-module logging baseline early, with enough context for end-to-end debugging.
- Check: smoke checks for key lifecycle, error, and degraded-state events.

7. Mock Visibility
- Implement: strict mock-field marking using `mock_columns`.
- Check: tests for highlighted vs non-highlighted columns.

8. Historical Klines Loader
- Implement: loader for Binance monthly/daily 1s kline files.
- Check: parser tests + fixture load test per coin pair.

9. Shared Bars-to-Features Transform
- Implement: one shared transform for training and runtime cold-start.
- Check: deterministic tests (`same input -> same features`) + schema/version check.

10. Model Training Pipeline
- Implement: offline training pipeline using historical klines and the shared bars-to-features transform; produce a model artifact.
- Check: training script runs end-to-end and emits a valid artifact.

11. Live Binance WS Transport Reliability
- Implement: Binance websocket connection lifecycle (connect/reconnect, stream continuity checks, degraded flags).
- Check: integration tests for reconnect and continuity behavior.

12. Live Binance WS Reducers (Midprice + 1s Klines)
- Implement: reducers consuming WS updates to produce midprice updates and continuous sealed 1s klines.
- Check: integration tests confirming both outputs from the same WS flow.

13. Cold-Start Readiness Gate
- Implement: readiness gating so inference/action paths are not enabled until sequence state is initialized from historical+recent data.
- Check: readiness tests for pre/post warmup transitions.

14. Atomic Dist Batch Path
- Implement: 1-second inference cycle wiring plus all-coins/all-durations dist commit path with reject-on-partial-invalid.
- Check: unit tests for commit/reject and previous-valid-state retention, and cadence check for 1-second inference updates.

15. Probability Recompute Wiring
- Implement: recompute probabilities on price and dist updates using shared probability functions.
- Check: integration tests for `price -> prob` and `dist -> prob`.

16. Sequential Jump Enforcement
- Implement: enforce no intentional delay between jumps for all defined execution paths.
- Check: integration assertions for jump order across active path transitions.

17. PM Top-of-Book Ingest
- Implement: YES best bid/ask ingest via SDK path.
- Check: integration test + dashboard book columns live.

18. Degraded-Mode Action Rules
- Implement: explicit behavior when required inputs are stale/missing so action flow is safe and predictable.
- Check: integration tests for degraded input combinations and expected action suppression behavior.

19. Action Eval Triggering (Single-Market)
- Implement: trigger evaluation from price/dist/book updates at single-market granularity.
- Check: integration tests for each trigger source.

20. Global Send Flag
- Implement: runtime flag controlling real external send vs suppressed send.
- Check: tests verifying actions still compute while external send is blocked.

21. Logging Completeness Pass
- Implement: finalize logging coverage across all active modules and path jumps.
- Check: integration checks for end-to-end traceability across discovery/price/dist/prob/eval/send.

22. Functional E2E Gate
- Implement: end-to-end run for current scope.
- Check: all required paths execute, outputs are produced, no stage is silently skipped, and dashboard does not stall the pipeline.

### 6.2 Step gate rule
Before moving to the next step:
1. Step checks pass.
2. Scope remains contained (no future-phase design choices unless required).
3. If UI-visible, dashboard is run and manually validated in browser.

### 6.3 Operational requirements
- Health endpoints must be available.
- Discovery failure degrades gracefully with deterministic placeholders.
- Dashboard failures do not disrupt engine loop.
- A global runtime flag must control whether orders are actually sent or not.
  - When disabled, the full decision pipeline still runs but external order sending is suppressed.
- Two runnable scripts are required:
  - one script to run `engine` + `dashboard` together
  - one script to train the model

### 6.4 Logging requirements (high-level)
- Logging must be global across all modules.
- Logs must contain enough data to identify and analyze useful events end-to-end.
- Logging design must prioritize clear traceability of decisions, actions, failures, and degraded states.
- The spec does not lock implementation details (format, backend, or specific library choices).

### 6.5 Not Implemented Yet (Explicitly Out of Current Scope)
The following capabilities are explicitly deferred and must not be treated as phase-complete requirements yet:
- Polymarket past-trades analysis for baseline binary cross-entropy evaluation.
- Full replay capacity.
- Replay determinism.
- Risk analysis.
- Action evaluation over sets of markets (portfolio/set-level decisions), beyond single-market evaluation.
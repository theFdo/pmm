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
- Live high-frequency midprice input: Binance orderbook websocket channels.
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
- No duplicated logic or unnecessary redefinitions anywhere (DRY policy).
- Probability derivation logic must be implemented once and reused everywhere.
- Market metadata structures must come from SDK/library types and be reused across modules; do not redefine metadata models locally.

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

### 6.1 Phase order (high level)
1. Contracts and shared reducers/stores.
2. Discovery + dashboard visibility.
3. Binance price + 1s bars + feature builder.
4. Dist batch + probs + action triggers.
5. PM top-of-book ingestion.
6. Positions/offers + net profit.
7. Execution/routing functional integration.

### 6.2 End-of-phase gate (mandatory)
Before moving to next phase:
1. Unit tests green.
2. Integration tests green.
3. Dashboard running and manually reviewed in browser.
4. Mock/live markings validated.
5. Regression notes captured.

### 6.3 Testing requirements
Unit:
- slug generation (all durations/coins)
- 4h offset behavior
- formatter behavior
- probability boundaries

Integration:
- slug batch resolution and hydration
- placeholder behavior on resolve miss
- trigger paths:
  - price -> probs -> action_eval
  - dist -> probs -> action_eval
  - book -> action_eval

E2E functional:
- end-to-end functional correctness: all required paths run, outputs are produced, and no stage is silently skipped
- dashboard on/off must not break or stall the execution pipeline

### 6.4 Order proposal and scope-control rules
- One implementation step at a time, phase aligned.
- Prefer simplest working design; avoid premature abstractions.
- Do not implement future-phase features early unless required by current phase.
- Keep placeholders explicit and marked.

### 6.5 Operational requirements
- Health endpoints must be available.
- Discovery failure degrades gracefully with deterministic placeholders.
- Dashboard failures do not disrupt engine loop.
- A global runtime flag must control whether orders are actually sent or not.
  - When disabled, the full decision pipeline still runs but external order sending is suppressed.
- Two runnable scripts are required:
  - one script to run `engine` + `dashboard` together
  - one script to train the model

### 6.6 Logging requirements (high-level)
- Logging must be global across all modules.
- Logs must contain enough data to identify and analyze useful events end-to-end.
- Logging design must prioritize clear traceability of decisions, actions, failures, and degraded states.
- The spec does not lock implementation details (format, backend, or specific library choices).

### 6.7 Not Implemented Yet (Explicitly Out of Current Scope)
The following capabilities are explicitly deferred and must not be treated as phase-complete requirements yet:
- Polymarket past-trades analysis for baseline binary cross-entropy evaluation.
- Full replay capacity.
- Replay determinism.
- Risk analysis.
- Action evaluation over sets of markets (portfolio/set-level decisions), beyond single-market evaluation.
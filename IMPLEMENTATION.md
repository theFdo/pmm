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

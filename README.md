# PMM Core (Steps 1-4)

This crate currently implements:
- Step 1: deterministic Polymarket slug generation for `5m`, `15m`, `1h`, `4h`, `1d`
- Step 2: discovery-resolution row model with explicit `Resolved` vs `Unresolved` status
- Step 3: SSR dashboard core table with the current required columns
- Step 4: dashboard logic pack (filters, in-interval recompute, formatting, 250ms polling)

## Step 1 behavior
- Interval scheduling is aligned to `America/New_York` wall-clock boundaries.
- `5m`/`15m`/`1h` are equivalent to UTC modulo boundaries.
- `4h` uses ET `00/04/08/12/16/20` boundaries.
- `1d` uses ET noon-to-noon windows.
- Slug generation is pure and I/O-free.

## Step 2 behavior
- Discovery rows preserve input order.
- Duplicate slugs are deduplicated for fetch efficiency and then re-expanded.
- Missing markets become `Unresolved(NotFound)`; transport issues become `Unresolved(TransportError)`.
- Rows are never silently dropped.
- Interval scheduling is deterministic with `previous`, `active`, and `next` windows:
  - `5m`: rollover at `:00/:05/:10/...`
  - `15m`: rollover at `:00/:15/:30/:45`
  - `1h`: rollover at every `hh:00`
  - `4h`: ET-aligned blocks at `00/04/08/12/16/20` ET
  - `1d`: ET noon-to-noon (`12:00 ET` start, `12:00 ET` end next day)

## Dashboard behavior (Steps 3-4)
- Dashboard route: `GET /dashboard`
- Snapshot route: `GET /dashboard/snapshot`
- Table scope defaults to `4 coins x 5 durations x previous/active/next = 60` rows.
- Dashboard server uses live continuous discovery by default (refresh loop + SDK metadata hydration).
- Filter semantics:
  - Query params: `coin`, `duration`, `bets_open`, `in_interval`
  - OR within each filter group, AND across groups
  - Missing group means "all selected"
- `in_interval` is recomputed from timestamps using `start_ts_utc <= now_ts_utc < end_ts_utc`.
- `End` cells are converted to browser-local `hh:mm` time in client JS.
- Snapshot polling cadence is `250ms`.
- Live metadata fields mapped from Gamma include:
  - `bets_open` (from `accepting_orders` / `closed` / `active`)
  - `taker_fee_pct`, `maker_fee_pct`, `fee_exponent`, `reward_pct`
- Fee profile rule:
  - `feeType=crypto_15_min` with `feesEnabled=true` => taker `0.25`, maker `-0.05`, exponent `2`
  - missing `feeType` or `feesEnabled=false` => taker `0`, maker `0`, exponent `-`
  - current SDK `Market` payload may omit `feeType`; fallback treats `5m/15m` + `feesEnabled=true` as `crypto_15_min`
- `ref_price`, `price`, and `probability` remain placeholders (`-`) for now (not sourced from Gamma market metadata in this step).

Example filter URL:

```text
/dashboard?coin=BTC&duration=1h&bets_open=open&in_interval=yes
```

Example filtered snapshot URL:

```text
/dashboard/snapshot?coin=BTC&duration=1h&in_interval=yes
```

## Build and test
Run default unit tests (no network):

```bash
. "$HOME/.cargo/env"
cargo test
```

Run dashboard server:

```bash
. "$HOME/.cargo/env"
cargo run --bin dashboard_server
```

Use demo snapshot source instead of live discovery:

```bash
PMM_DASHBOARD_USE_DEMO=1 cargo run --bin dashboard_server
```

Run live Gamma integration test (network required):

```bash
. "$HOME/.cargo/env"
cargo test --features live-gamma-tests --test discovery_live_gamma
```

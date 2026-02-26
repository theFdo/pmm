# PMM Core (Steps 1-4)

This crate currently implements:
- Step 1: deterministic Polymarket slug generation for `5m`, `15m`, `1h`, `4h`
- Step 2: discovery-resolution row model with explicit `Resolved` vs `Unresolved` status
- Step 3: SSR dashboard core table with 22 required columns
- Step 4: dashboard logic pack (filters, in-interval recompute, formatting, 100ms polling)

## Step 1 behavior
- `1h` slugs use `America/New_York` wall-clock time (DST-aware).
- `4h` slugs are aligned with `PMFLIPS_DISCOVERY_OFFSET_4H_MIN` via `SlugConfig.discovery_offset_4h_min`.
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
  - `4h`: same modulo logic, shifted by `SlugConfig.discovery_offset_4h_min` (e.g. `60` => `01:00/05:00/09:00/...` UTC)

## Dashboard behavior (Steps 3-4)
- Dashboard route: `GET /dashboard`
- Snapshot route: `GET /dashboard/snapshot`
- Table scope defaults to `4 coins x 4 durations x previous/active/next = 48` rows.
- Filter semantics:
  - Query params: `coin`, `duration`, `bets_open`, `in_interval`
  - OR within each filter group, AND across groups
  - Missing group means "all selected"
- `in_interval` is recomputed from timestamps using `start_ts_utc <= now_ts_utc < end_ts_utc`.
- `End` cells are converted to browser-local `hh:mm` time in client JS.
- Snapshot polling cadence is `100ms`.

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

Run live Gamma integration test (network required):

```bash
. "$HOME/.cargo/env"
# optional: export PMFLIPS_DISCOVERY_OFFSET_4H_MIN=60
cargo test --features live-gamma-tests --test discovery_live_gamma
```

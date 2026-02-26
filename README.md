# PMM Core (Steps 1-2)

This crate currently implements:
- Step 1: deterministic Polymarket slug generation for `5m`, `15m`, `1h`, `4h`
- Step 2: discovery-resolution row model with explicit `Resolved` vs `Unresolved` status

## Step 1 behavior
- `1h` slugs use `America/New_York` wall-clock time (DST-aware).
- `4h` slugs are aligned with `PMFLIPS_DISCOVERY_OFFSET_4H_MIN` via `SlugConfig.discovery_offset_4h_min`.
- Slug generation is pure and I/O-free.

## Step 2 behavior
- Discovery rows preserve input order.
- Duplicate slugs are deduplicated for fetch efficiency and then re-expanded.
- Missing markets become `Unresolved(NotFound)`; transport issues become `Unresolved(TransportError)`.
- Rows are never silently dropped.

## Build and test
Run default unit tests (no network):

```bash
. "$HOME/.cargo/env"
cargo test
```

Run live Gamma integration test (network required):

```bash
. "$HOME/.cargo/env"
cargo test --features live-gamma-tests --test discovery_live_gamma
```

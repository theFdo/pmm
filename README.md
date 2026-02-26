# PMM Slug Generation (Step 1)

This project currently implements deterministic Polymarket slug generation for:
- `5m`, `15m`, `1h`, `4h`
- coins `BTC`, `ETH`, `SOL`, `XRP`

Behavior notes:
- `1h` slugs are rendered from `America/New_York` wall-clock time (DST-aware).
- `4h` slugs are aligned using `PMFLIPS_DISCOVERY_OFFSET_4H_MIN` via `SlugConfig.discovery_offset_4h_min`.
- generation is pure and I/O-free.

Run tests:

```bash
cargo test
```

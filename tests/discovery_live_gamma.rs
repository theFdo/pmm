#![cfg(feature = "live-gamma-tests")]

use chrono::Utc;
use pmm::{
    resolve_discovery_batch, Coin, DiscoveryConfig, DiscoveryKey, DiscoveryStatus, Duration,
    SlugConfig,
};
use tokio::time::{sleep, Duration as TokioDuration};

fn interval_seconds(duration: Duration) -> i64 {
    match duration {
        Duration::M5 => 5 * 60,
        Duration::M15 => 15 * 60,
        Duration::H1 => 60 * 60,
        Duration::H4 => 4 * 60 * 60,
    }
}

fn ceil_to_interval(ts: i64, step: i64) -> i64 {
    let q = (ts + step - 1).div_euclid(step);
    q * step
}

fn candidate_end_timestamps(now_ts: i64, duration: Duration) -> Vec<i64> {
    let step = interval_seconds(duration);
    let mut out = Vec::new();

    for k in -8..=8 {
        let shifted = now_ts + (k as i64) * step;
        out.push(ceil_to_interval(shifted, step));
    }

    out.sort_unstable();
    out.dedup();
    out
}

#[tokio::test]
async fn live_gamma_returns_found_and_unresolved_rows() {
    let slug_cfg = SlugConfig::default();
    let discovery_cfg = DiscoveryConfig {
        timeout_ms: 8_000,
        batch_size: 32,
        max_retries: 2,
        retry_backoff_ms: 300,
        include_tag: false,
    };

    let coins = [Coin::Btc, Coin::Eth, Coin::Sol, Coin::Xrp];
    let durations = [Duration::M5, Duration::M15, Duration::H1, Duration::H4];

    let now_ts = Utc::now().timestamp();
    let mut keys = Vec::new();

    for coin in coins {
        for duration in durations {
            for end_ts in candidate_end_timestamps(now_ts, duration) {
                let key = DiscoveryKey::new(coin, duration, end_ts, slug_cfg)
                    .expect("discovery key build should succeed");
                keys.push(key);
            }
        }
    }

    let invalid_slug = "pmm-invalid-slug-should-not-exist-000000000";
    keys.push(DiscoveryKey::from_slug(
        Coin::Btc,
        Duration::M5,
        now_ts,
        invalid_slug,
    ));

    let mut last_resolved_count = 0usize;
    let mut attempts = 0;

    while attempts < 3 {
        attempts += 1;

        let rows = resolve_discovery_batch(&keys, &discovery_cfg)
            .await
            .expect("live gamma call should return rows");

        assert_eq!(rows.len(), keys.len(), "no rows should be dropped");

        let resolved_count = rows
            .iter()
            .filter(|row| matches!(row.status, DiscoveryStatus::Resolved { .. }))
            .count();
        last_resolved_count = resolved_count;

        let invalid_is_unresolved = rows.iter().any(|row| {
            row.key.slug == invalid_slug && matches!(row.status, DiscoveryStatus::Unresolved { .. })
        });

        if resolved_count > 0 && invalid_is_unresolved {
            return;
        }

        sleep(TokioDuration::from_secs(2)).await;
    }

    panic!(
        "did not observe both expected conditions after retries: resolved_count={}, invalid_slug_unresolved expected=true",
        last_resolved_count
    );
}

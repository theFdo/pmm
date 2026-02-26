#![cfg(feature = "live-gamma-tests")]

use chrono::Utc;
use pmm::{
    build_previous_active_and_next_discovery_keys, resolve_discovery_batch, DiscoveryConfig,
    DiscoveryKey, DiscoveryStatus, SlugConfig, ALL_COINS, ALL_DURATIONS,
};
use tokio::time::{sleep, Duration as TokioDuration};

fn slug_config_from_env() -> SlugConfig {
    let offset = std::env::var("PMFLIPS_DISCOVERY_OFFSET_4H_MIN")
        .ok()
        .and_then(|raw| raw.parse::<i32>().ok())
        .unwrap_or(60);

    SlugConfig {
        discovery_offset_4h_min: offset,
    }
}

fn live_discovery_config() -> DiscoveryConfig {
    DiscoveryConfig {
        timeout_ms: 8_000,
        batch_size: 64,
        max_retries: 2,
        retry_backoff_ms: 300,
        include_tag: false,
    }
}

fn scheduled_keys(now_ts: i64, slug_cfg: SlugConfig) -> Vec<DiscoveryKey> {
    build_previous_active_and_next_discovery_keys(now_ts, &ALL_COINS, &ALL_DURATIONS, slug_cfg)
        .expect("scheduled discovery key build should succeed")
        .into_iter()
        .map(|scheduled| scheduled.key)
        .collect()
}

#[tokio::test]
async fn live_gamma_confirms_all_scheduled_slugs_exist() {
    let now_ts = Utc::now().timestamp();
    let slug_cfg = slug_config_from_env();
    let discovery_cfg = live_discovery_config();
    let keys = scheduled_keys(now_ts, slug_cfg);

    let mut last_unresolved: Vec<String> = Vec::new();

    for _attempt in 0..5 {
        let rows = resolve_discovery_batch(&keys, &discovery_cfg)
            .await
            .expect("live gamma call should return rows");

        assert_eq!(rows.len(), keys.len(), "no rows should be dropped");

        let unresolved: Vec<String> = rows
            .iter()
            .filter_map(|row| match &row.status {
                DiscoveryStatus::Resolved { .. } => None,
                DiscoveryStatus::Unresolved { .. } => Some(row.key.slug.clone()),
            })
            .collect();

        if unresolved.is_empty() {
            return;
        }

        last_unresolved = unresolved;
        sleep(TokioDuration::from_secs(2)).await;
    }

    panic!(
        "some scheduled slugs did not resolve via Gamma (count={}): {:?}",
        last_unresolved.len(),
        &last_unresolved[..last_unresolved.len().min(12)]
    );
}

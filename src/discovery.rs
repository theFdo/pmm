//! Step 2 discovery resolution: resolve deterministic slugs into Polymarket metadata.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Days, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;
use thiserror::Error;

use crate::{build_slug, Coin, Duration, SlugConfig, SlugError};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiscoveryKey {
    pub coin: Coin,
    pub duration: Duration,
    pub start_ts_utc: i64,
    pub slug: String,
}

impl DiscoveryKey {
    pub fn new(
        coin: Coin,
        duration: Duration,
        start_ts_utc: i64,
        slug_cfg: SlugConfig,
    ) -> Result<Self, SlugError> {
        Ok(Self {
            coin,
            duration,
            start_ts_utc,
            slug: build_slug(coin, duration, start_ts_utc, slug_cfg)?,
        })
    }

    pub fn from_slug(
        coin: Coin,
        duration: Duration,
        start_ts_utc: i64,
        slug: impl Into<String>,
    ) -> Self {
        Self {
            coin,
            duration,
            start_ts_utc,
            slug: slug.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryRow<M> {
    pub key: DiscoveryKey,
    pub status: DiscoveryStatus<M>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryStatus<M> {
    Resolved { market: M },
    Unresolved { reason: UnresolvedReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    NotFound,
    TransportError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryConfig {
    pub timeout_ms: u64,
    pub batch_size: usize,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub include_tag: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 3_000,
            batch_size: 64,
            max_retries: 2,
            retry_backoff_ms: 200,
            include_tag: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugFetchOutcome<M> {
    Found(M),
    Missing,
    TransportError(String),
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("invalid discovery config: batch_size must be >= 1")]
    InvalidBatchSize,
    #[error("discovery transport error: {0}")]
    Transport(String),
}

pub const ALL_COINS: [Coin; 4] = [Coin::Btc, Coin::Eth, Coin::Sol, Coin::Xrp];
pub const ALL_DURATIONS: [Duration; 5] = [
    Duration::M5,
    Duration::M15,
    Duration::H1,
    Duration::H4,
    Duration::D1,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscoveryWindow {
    Previous,
    Active,
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledDiscoveryKey {
    pub window: DiscoveryWindow,
    pub key: DiscoveryKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalStarts {
    pub previous_start_ts_utc: i64,
    pub active_start_ts_utc: i64,
    pub next_start_ts_utc: i64,
}

pub fn interval_starts_for_now(
    duration: Duration,
    now_ts_utc: i64,
    slug_cfg: SlugConfig,
) -> IntervalStarts {
    if matches!(duration, Duration::H4) {
        return interval_starts_for_4h_ny(now_ts_utc);
    }
    if matches!(duration, Duration::D1) {
        return interval_starts_for_1d_ny_noon(now_ts_utc);
    }

    let step = duration_step_seconds(duration);
    let offset = interval_offset_seconds(duration, slug_cfg);
    let aligned_base = now_ts_utc.saturating_sub(offset);
    let active_start = aligned_base
        .div_euclid(step)
        .saturating_mul(step)
        .saturating_add(offset);
    let previous_start_ts_utc = active_start.saturating_sub(step);
    let next_start_ts_utc = active_start.saturating_add(step);

    IntervalStarts {
        previous_start_ts_utc,
        active_start_ts_utc: active_start,
        next_start_ts_utc,
    }
}

fn interval_starts_for_4h_ny(now_ts_utc: i64) -> IntervalStarts {
    let now_ny = Utc
        .timestamp_opt(now_ts_utc, 0)
        .single()
        .expect("valid now timestamp expected")
        .with_timezone(&New_York);
    let date = now_ny.date_naive();

    let block_hour = (now_ny.hour() / 4) * 4;
    let active = ny_datetime(date, block_hour).expect("valid NY local 4h boundary");

    let previous = if block_hour == 0 {
        let prev_date = date
            .checked_sub_days(Days::new(1))
            .expect("previous date should exist");
        ny_datetime(prev_date, 20).expect("valid previous NY 4h boundary")
    } else {
        ny_datetime(date, block_hour - 4).expect("valid previous NY 4h boundary")
    };

    let next = if block_hour == 20 {
        let next_date = date
            .checked_add_days(Days::new(1))
            .expect("next date should exist");
        ny_datetime(next_date, 0).expect("valid next NY 4h boundary")
    } else {
        ny_datetime(date, block_hour + 4).expect("valid next NY 4h boundary")
    };

    IntervalStarts {
        previous_start_ts_utc: previous.with_timezone(&Utc).timestamp(),
        active_start_ts_utc: active.with_timezone(&Utc).timestamp(),
        next_start_ts_utc: next.with_timezone(&Utc).timestamp(),
    }
}

fn interval_starts_for_1d_ny_noon(now_ts_utc: i64) -> IntervalStarts {
    let now_ny = Utc
        .timestamp_opt(now_ts_utc, 0)
        .single()
        .expect("valid now timestamp expected")
        .with_timezone(&New_York);
    let date = now_ny.date_naive();
    let today_noon = ny_datetime(date, 12).expect("valid NY noon");

    let active = if now_ny < today_noon {
        let prev_date = date
            .checked_sub_days(Days::new(1))
            .expect("previous date should exist");
        ny_datetime(prev_date, 12).expect("valid previous NY noon")
    } else {
        today_noon
    };

    let active_date = active.date_naive();
    let previous_date = active_date
        .checked_sub_days(Days::new(1))
        .expect("previous date should exist");
    let next_date = active_date
        .checked_add_days(Days::new(1))
        .expect("next date should exist");

    let previous = ny_datetime(previous_date, 12).expect("valid previous NY noon");
    let next = ny_datetime(next_date, 12).expect("valid next NY noon");

    IntervalStarts {
        previous_start_ts_utc: previous.with_timezone(&Utc).timestamp(),
        active_start_ts_utc: active.with_timezone(&Utc).timestamp(),
        next_start_ts_utc: next.with_timezone(&Utc).timestamp(),
    }
}

fn ny_datetime(date: chrono::NaiveDate, hour: u32) -> Option<chrono::DateTime<chrono_tz::Tz>> {
    New_York
        .with_ymd_and_hms(date.year(), date.month(), date.day(), hour, 0, 0)
        .single()
}

pub fn build_active_discovery_keys(
    now_ts_utc: i64,
    coins: &[Coin],
    durations: &[Duration],
    slug_cfg: SlugConfig,
) -> Result<Vec<DiscoveryKey>, SlugError> {
    let mut keys = Vec::with_capacity(coins.len() * durations.len());

    for duration in durations {
        let starts = interval_starts_for_now(*duration, now_ts_utc, slug_cfg);
        for coin in coins {
            keys.push(DiscoveryKey::new(
                *coin,
                *duration,
                starts.active_start_ts_utc,
                slug_cfg,
            )?);
        }
    }

    Ok(keys)
}

pub fn build_active_and_next_discovery_keys(
    now_ts_utc: i64,
    coins: &[Coin],
    durations: &[Duration],
    slug_cfg: SlugConfig,
) -> Result<Vec<ScheduledDiscoveryKey>, SlugError> {
    let all =
        build_previous_active_and_next_discovery_keys(now_ts_utc, coins, durations, slug_cfg)?;
    let keys = all
        .into_iter()
        .filter(|scheduled| {
            matches!(
                scheduled.window,
                DiscoveryWindow::Active | DiscoveryWindow::Next
            )
        })
        .collect();

    Ok(keys)
}

pub fn build_previous_active_and_next_discovery_keys(
    now_ts_utc: i64,
    coins: &[Coin],
    durations: &[Duration],
    slug_cfg: SlugConfig,
) -> Result<Vec<ScheduledDiscoveryKey>, SlugError> {
    let mut keys = Vec::with_capacity(coins.len() * durations.len() * 3);

    for duration in durations {
        let starts = interval_starts_for_now(*duration, now_ts_utc, slug_cfg);
        for coin in coins {
            keys.push(ScheduledDiscoveryKey {
                window: DiscoveryWindow::Previous,
                key: DiscoveryKey::new(*coin, *duration, starts.previous_start_ts_utc, slug_cfg)?,
            });
            keys.push(ScheduledDiscoveryKey {
                window: DiscoveryWindow::Active,
                key: DiscoveryKey::new(*coin, *duration, starts.active_start_ts_utc, slug_cfg)?,
            });
            keys.push(ScheduledDiscoveryKey {
                window: DiscoveryWindow::Next,
                key: DiscoveryKey::new(*coin, *duration, starts.next_start_ts_utc, slug_cfg)?,
            });
        }
    }

    Ok(keys)
}

fn duration_step_seconds(duration: Duration) -> i64 {
    match duration {
        Duration::M5 => 5 * 60,
        Duration::M15 => 15 * 60,
        Duration::H1 => 60 * 60,
        Duration::H4 => 4 * 60 * 60,
        Duration::D1 => 24 * 60 * 60,
    }
}

fn interval_offset_seconds(duration: Duration, slug_cfg: SlugConfig) -> i64 {
    match duration {
        Duration::H4 => {
            let _ = slug_cfg;
            0
        }
        Duration::M5 | Duration::M15 | Duration::H1 | Duration::D1 => 0,
    }
}

pub fn resolve_discovery_batch_with_fetcher<M, F>(
    keys: &[DiscoveryKey],
    cfg: &DiscoveryConfig,
    mut fetcher: F,
) -> Result<Vec<DiscoveryRow<M>>, DiscoveryError>
where
    M: Clone,
    F: FnMut(&[String]) -> Result<HashMap<String, SlugFetchOutcome<M>>, DiscoveryError>,
{
    if cfg.batch_size == 0 {
        return Err(DiscoveryError::InvalidBatchSize);
    }

    let unique_slugs = ordered_unique_slugs(keys);
    let mut slug_outcomes: HashMap<String, SlugFetchOutcome<M>> =
        HashMap::with_capacity(unique_slugs.len());

    for chunk in unique_slugs.chunks(cfg.batch_size) {
        let fetched = fetcher(chunk)?;
        for slug in chunk {
            let outcome = fetched
                .get(slug)
                .cloned()
                .unwrap_or(SlugFetchOutcome::Missing);
            slug_outcomes.insert(slug.clone(), outcome);
        }
    }

    Ok(materialize_rows(keys, &slug_outcomes))
}

fn ordered_unique_slugs(keys: &[DiscoveryKey]) -> Vec<String> {
    let mut seen = HashSet::with_capacity(keys.len());
    let mut unique = Vec::with_capacity(keys.len());

    for key in keys {
        if seen.insert(key.slug.clone()) {
            unique.push(key.slug.clone());
        }
    }

    unique
}

fn materialize_rows<M: Clone>(
    keys: &[DiscoveryKey],
    slug_outcomes: &HashMap<String, SlugFetchOutcome<M>>,
) -> Vec<DiscoveryRow<M>> {
    keys.iter()
        .map(|key| {
            let status = match slug_outcomes.get(&key.slug) {
                Some(SlugFetchOutcome::Found(market)) => DiscoveryStatus::Resolved {
                    market: market.clone(),
                },
                Some(SlugFetchOutcome::TransportError(message)) => DiscoveryStatus::Unresolved {
                    reason: UnresolvedReason::TransportError(message.clone()),
                },
                Some(SlugFetchOutcome::Missing) | None => DiscoveryStatus::Unresolved {
                    reason: UnresolvedReason::NotFound,
                },
            };

            DiscoveryRow {
                key: key.clone(),
                status,
            }
        })
        .collect()
}

#[cfg(feature = "discovery-sdk")]
pub type SdkMarket = polymarket_client_sdk::gamma::types::response::Market;

#[cfg(feature = "discovery-sdk")]
pub async fn resolve_discovery_batch(
    keys: &[DiscoveryKey],
    cfg: &DiscoveryConfig,
) -> Result<Vec<DiscoveryRow<SdkMarket>>, DiscoveryError> {
    use polymarket_client_sdk::gamma::Client as GammaClient;

    if cfg.batch_size == 0 {
        return Err(DiscoveryError::InvalidBatchSize);
    }

    let client = GammaClient::default();
    let unique_slugs = ordered_unique_slugs(keys);
    let mut slug_outcomes: HashMap<String, SlugFetchOutcome<SdkMarket>> =
        HashMap::with_capacity(unique_slugs.len());

    for chunk in unique_slugs.chunks(cfg.batch_size) {
        for slug in chunk {
            let outcome = fetch_market_by_slug_with_retry(&client, slug, cfg).await;
            slug_outcomes.insert(slug.clone(), outcome);
        }
    }

    let has_non_transport = slug_outcomes
        .values()
        .any(|outcome| !matches!(outcome, SlugFetchOutcome::TransportError(_)));
    if !keys.is_empty() && !has_non_transport {
        return Err(DiscoveryError::Transport(
            "all slug lookups failed with transport errors".to_string(),
        ));
    }

    Ok(materialize_rows(keys, &slug_outcomes))
}

#[cfg(feature = "discovery-sdk")]
async fn fetch_market_by_slug_with_retry(
    client: &polymarket_client_sdk::gamma::Client,
    slug: &str,
    cfg: &DiscoveryConfig,
) -> SlugFetchOutcome<SdkMarket> {
    use polymarket_client_sdk::gamma::types::request::MarketBySlugRequest;
    use tokio::time::{sleep, timeout, Duration};

    let mut attempt: u32 = 0;

    loop {
        let request = MarketBySlugRequest::builder()
            .slug(slug.to_string())
            .include_tag(cfg.include_tag)
            .build();

        let call_result = timeout(
            Duration::from_millis(cfg.timeout_ms),
            client.market_by_slug(&request),
        )
        .await;

        match call_result {
            Ok(Ok(market)) => return SlugFetchOutcome::Found(market),
            Ok(Err(err)) if is_not_found_error(&err) => return SlugFetchOutcome::Missing,
            Ok(Err(err)) => {
                if attempt >= cfg.max_retries {
                    return SlugFetchOutcome::TransportError(err.to_string());
                }
            }
            Err(_) => {
                if attempt >= cfg.max_retries {
                    return SlugFetchOutcome::TransportError(format!(
                        "timeout after {}ms while resolving slug {}",
                        cfg.timeout_ms, slug
                    ));
                }
            }
        }

        attempt += 1;
        sleep(backoff_duration(cfg.retry_backoff_ms, attempt)).await;
    }
}

#[cfg(feature = "discovery-sdk")]
fn backoff_duration(base_ms: u64, attempt: u32) -> std::time::Duration {
    let shift = attempt.saturating_sub(1).min(10);
    let factor = 1u64 << shift;
    std::time::Duration::from_millis(base_ms.saturating_mul(factor))
}

#[cfg(feature = "discovery-sdk")]
fn is_not_found_error(err: &polymarket_client_sdk::error::Error) -> bool {
    use polymarket_client_sdk::error::{Kind, Status, StatusCode};

    if err.kind() != Kind::Status {
        return false;
    }

    err.downcast_ref::<Status>()
        .map(|status| status.status_code == StatusCode::NOT_FOUND)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeMarket {
        slug: String,
    }

    fn key(slug: &str) -> DiscoveryKey {
        DiscoveryKey::from_slug(Coin::Btc, Duration::M5, 1_735_689_600, slug.to_string())
    }

    #[test]
    fn preserves_input_order_after_dedupe_and_reexpand() {
        let keys = vec![key("slug-b"), key("slug-a"), key("slug-b")];
        let cfg = DiscoveryConfig {
            batch_size: 16,
            ..DiscoveryConfig::default()
        };

        let rows = resolve_discovery_batch_with_fetcher(&keys, &cfg, |slugs| {
            let mut out = HashMap::new();
            for slug in slugs {
                out.insert(
                    slug.clone(),
                    SlugFetchOutcome::Found(FakeMarket { slug: slug.clone() }),
                );
            }
            Ok(out)
        })
        .unwrap();

        assert_eq!(rows.len(), keys.len());
        assert_eq!(rows[0].key.slug, "slug-b");
        assert_eq!(rows[1].key.slug, "slug-a");
        assert_eq!(rows[2].key.slug, "slug-b");
    }

    #[test]
    fn duplicate_slugs_are_fetched_once() {
        let keys = vec![key("same"), key("same"), key("other"), key("same")];
        let cfg = DiscoveryConfig {
            batch_size: 16,
            ..DiscoveryConfig::default()
        };

        let mut seen: HashMap<String, usize> = HashMap::new();

        let rows = resolve_discovery_batch_with_fetcher(&keys, &cfg, |slugs| {
            let mut out = HashMap::new();
            for slug in slugs {
                *seen.entry(slug.clone()).or_insert(0) += 1;
                out.insert(
                    slug.clone(),
                    SlugFetchOutcome::Found(FakeMarket { slug: slug.clone() }),
                );
            }
            Ok(out)
        })
        .unwrap();

        assert_eq!(rows.len(), keys.len());
        assert_eq!(seen.get("same"), Some(&1));
        assert_eq!(seen.get("other"), Some(&1));
    }

    #[test]
    fn missing_slug_becomes_unresolved_not_found() {
        let keys = vec![key("found"), key("missing")];
        let cfg = DiscoveryConfig::default();

        let rows = resolve_discovery_batch_with_fetcher(&keys, &cfg, |_slugs| {
            let mut out = HashMap::new();
            out.insert(
                "found".to_string(),
                SlugFetchOutcome::Found(FakeMarket {
                    slug: "found".to_string(),
                }),
            );
            Ok(out)
        })
        .unwrap();

        assert!(matches!(
            rows[0].status,
            DiscoveryStatus::Resolved {
                market: FakeMarket { .. }
            }
        ));
        assert_eq!(
            rows[1].status,
            DiscoveryStatus::Unresolved {
                reason: UnresolvedReason::NotFound
            }
        );
    }

    #[test]
    fn transport_error_becomes_unresolved_transport() {
        let keys = vec![key("bad-slug")];
        let cfg = DiscoveryConfig::default();

        let rows = resolve_discovery_batch_with_fetcher(&keys, &cfg, |_slugs| {
            let mut out = HashMap::new();
            out.insert(
                "bad-slug".to_string(),
                SlugFetchOutcome::<FakeMarket>::TransportError("timeout".to_string()),
            );
            Ok(out)
        })
        .unwrap();

        assert_eq!(
            rows[0].status,
            DiscoveryStatus::Unresolved {
                reason: UnresolvedReason::TransportError("timeout".to_string())
            }
        );
    }

    #[test]
    fn no_rows_are_dropped_when_unresolved() {
        let keys = vec![key("a"), key("b"), key("c")];
        let cfg = DiscoveryConfig::default();

        let rows = resolve_discovery_batch_with_fetcher::<FakeMarket, _>(&keys, &cfg, |_slugs| {
            Ok(HashMap::new())
        })
        .unwrap();
        assert_eq!(rows.len(), keys.len());
        assert!(rows
            .iter()
            .all(|row| matches!(row.status, DiscoveryStatus::Unresolved { .. })));
    }

    #[test]
    fn batch_size_zero_is_rejected() {
        let keys = vec![key("a")];
        let cfg = DiscoveryConfig {
            batch_size: 0,
            ..DiscoveryConfig::default()
        };

        let err = resolve_discovery_batch_with_fetcher::<FakeMarket, _>(&keys, &cfg, |_slugs| {
            Ok(HashMap::new())
        })
        .unwrap_err();

        assert!(matches!(err, DiscoveryError::InvalidBatchSize));
    }

    #[test]
    fn interval_5m_boundary_rolls_to_next_start() {
        // 2025-01-01 00:05:00 UTC
        let now = 1_735_689_900;
        let starts = interval_starts_for_now(Duration::M5, now, SlugConfig::default());
        assert_eq!(starts.previous_start_ts_utc, 1_735_689_600); // 00:00
        assert_eq!(starts.active_start_ts_utc, 1_735_689_900); // 00:05
        assert_eq!(starts.next_start_ts_utc, 1_735_690_200); // 00:10
    }

    #[test]
    fn interval_15m_boundary_rolls_to_next_start() {
        // 2025-01-01 00:15:00 UTC
        let now = 1_735_690_500;
        let starts = interval_starts_for_now(Duration::M15, now, SlugConfig::default());
        assert_eq!(starts.previous_start_ts_utc, 1_735_689_600); // 00:00
        assert_eq!(starts.active_start_ts_utc, 1_735_690_500); // 00:15
        assert_eq!(starts.next_start_ts_utc, 1_735_691_400); // 00:30
    }

    #[test]
    fn interval_1h_boundary_rolls_to_next_hour_start() {
        // 2025-01-01 13:00:00 UTC
        let now = 1_735_736_400;
        let starts = interval_starts_for_now(Duration::H1, now, SlugConfig::default());
        assert_eq!(starts.previous_start_ts_utc, 1_735_732_800); // 12:00
        assert_eq!(starts.active_start_ts_utc, 1_735_736_400); // 13:00
        assert_eq!(starts.next_start_ts_utc, 1_735_740_000); // 14:00
    }

    #[test]
    fn interval_4h_uses_et_previous_active_and_next_start() {
        let cfg = SlugConfig::default();
        // 2025-01-01 00:30:00 UTC = 2024-12-31 19:30 ET
        // ET 4h boundaries are 00/04/08/12/16/20 ET.
        let now = 1_735_691_400;
        let starts = interval_starts_for_now(Duration::H4, now, cfg);
        let previous = ny_datetime(chrono::NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(), 12)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let active = ny_datetime(chrono::NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(), 16)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let next = ny_datetime(chrono::NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(), 20)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();

        assert_eq!(starts.previous_start_ts_utc, previous);
        assert_eq!(starts.active_start_ts_utc, active);
        assert_eq!(starts.next_start_ts_utc, next);
    }

    #[test]
    fn interval_1d_aligns_to_et_noon_boundaries() {
        let cfg = SlugConfig::default();
        // 2026-02-26 16:20:00 UTC = 11:20 ET, so active window started previous day at 12:00 ET.
        let now = 1_772_122_800;
        let starts = interval_starts_for_now(Duration::D1, now, cfg);

        let previous = ny_datetime(chrono::NaiveDate::from_ymd_opt(2026, 2, 24).unwrap(), 12)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let active = ny_datetime(chrono::NaiveDate::from_ymd_opt(2026, 2, 25).unwrap(), 12)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();
        let next = ny_datetime(chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap(), 12)
            .unwrap()
            .with_timezone(&Utc)
            .timestamp();

        assert_eq!(starts.previous_start_ts_utc, previous);
        assert_eq!(starts.active_start_ts_utc, active);
        assert_eq!(starts.next_start_ts_utc, next);
    }

    #[test]
    fn active_discovery_keys_use_current_active_interval_start() {
        let now = 1_735_689_900; // 2025-01-01 00:05:00 UTC
        let keys =
            build_active_discovery_keys(now, &ALL_COINS, &[Duration::M5], SlugConfig::default())
                .unwrap();
        assert_eq!(keys.len(), ALL_COINS.len());
        assert_eq!(keys[0].start_ts_utc, 1_735_689_900); // 00:05
        assert_eq!(keys[0].slug, "btc-updown-5m-1735689900");
    }

    #[test]
    fn active_and_next_key_builder_covers_all_pairs() {
        let now = 1_735_689_901;
        let scheduled = build_active_and_next_discovery_keys(
            now,
            &ALL_COINS,
            &ALL_DURATIONS,
            SlugConfig::default(),
        )
        .unwrap();
        assert_eq!(scheduled.len(), ALL_COINS.len() * ALL_DURATIONS.len() * 2);
        assert!(matches!(scheduled[0].window, DiscoveryWindow::Active));
        assert!(matches!(scheduled[1].window, DiscoveryWindow::Next));
    }

    #[test]
    fn previous_active_next_key_builder_covers_all_pairs() {
        let now = 1_735_689_901;
        let scheduled = build_previous_active_and_next_discovery_keys(
            now,
            &ALL_COINS,
            &ALL_DURATIONS,
            SlugConfig::default(),
        )
        .unwrap();
        assert_eq!(scheduled.len(), ALL_COINS.len() * ALL_DURATIONS.len() * 3);
        assert!(matches!(scheduled[0].window, DiscoveryWindow::Previous));
        assert!(matches!(scheduled[1].window, DiscoveryWindow::Active));
        assert!(matches!(scheduled[2].window, DiscoveryWindow::Next));
    }
}

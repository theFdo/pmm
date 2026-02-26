//! Deterministic slug generation for Polymarket up/down markets.
//!
//! Rules implemented:
//! - 5m/15m/4h: `{coin_short}-updown-{duration}-{end_ts_s}`
//! - 1h: `{coin_full}-up-or-down-{month}-{day}-{hour12}{am_pm}-et`
//! - 1d: `{coin_full}-up-or-down-on-{month}-{day}`
//! - 1h formatting uses America/New_York wall-clock time (DST-aware)
//! - 1d formatting uses the market resolution date in America/New_York
//! - 4h alignment uses America/New_York wall-clock 4h boundaries

use chrono::{DateTime, Datelike, Days, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coin {
    Btc,
    Eth,
    Sol,
    Xrp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Duration {
    M5,
    M15,
    H1,
    H4,
    D1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlugConfig {
    pub discovery_offset_4h_min: i32,
}

impl Default for SlugConfig {
    fn default() -> Self {
        Self {
            discovery_offset_4h_min: 0,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SlugError {
    #[error("unsupported coin: {0}")]
    UnsupportedCoin(String),
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error("invalid unix timestamp: {0}")]
    InvalidTimestamp(i64),
}

pub fn parse_coin(input: &str) -> Result<Coin, SlugError> {
    match input {
        "BTC" => Ok(Coin::Btc),
        "ETH" => Ok(Coin::Eth),
        "SOL" => Ok(Coin::Sol),
        "XRP" => Ok(Coin::Xrp),
        other => Err(SlugError::UnsupportedCoin(other.to_string())),
    }
}

pub fn parse_duration(input: &str) -> Result<Duration, SlugError> {
    match input {
        "5m" => Ok(Duration::M5),
        "15m" => Ok(Duration::M15),
        "1h" => Ok(Duration::H1),
        "4h" => Ok(Duration::H4),
        "1d" => Ok(Duration::D1),
        other => Err(SlugError::InvalidDuration(other.to_string())),
    }
}

pub fn build_slug(
    coin: Coin,
    duration: Duration,
    end_ts_utc: i64,
    cfg: SlugConfig,
) -> Result<String, SlugError> {
    let _ = cfg;
    let _ = utc_from_ts(end_ts_utc)?;

    match duration {
        Duration::M5 => Ok(format!("{}-updown-5m-{}", coin_short(coin), end_ts_utc)),
        Duration::M15 => Ok(format!("{}-updown-15m-{}", coin_short(coin), end_ts_utc)),
        Duration::H4 => {
            let aligned = align_4h_start_ts_et(end_ts_utc)?;
            Ok(format!("{}-updown-4h-{}", coin_short(coin), aligned))
        }
        Duration::H1 => {
            let ny = utc_from_ts(end_ts_utc)?.with_timezone(&New_York);
            let month = ny.format("%B").to_string().to_lowercase();
            let day = ny.format("%-d");
            let hour = ny.format("%-I");
            let am_pm = ny.format("%P");
            Ok(format!(
                "{}-up-or-down-{}-{}-{}{}-et",
                coin_full(coin),
                month,
                day,
                hour,
                am_pm
            ))
        }
        Duration::D1 => {
            let start_ny = utc_from_ts(end_ts_utc)?.with_timezone(&New_York);
            let resolution_date = start_ny
                .date_naive()
                .checked_add_days(Days::new(1))
                .ok_or(SlugError::InvalidTimestamp(end_ts_utc))?;
            let resolution_noon_ny =
                ny_noon_for_date(resolution_date).ok_or(SlugError::InvalidTimestamp(end_ts_utc))?;
            let month = resolution_noon_ny.format("%B").to_string().to_lowercase();
            let day = resolution_noon_ny.format("%-d");
            Ok(format!(
                "{}-up-or-down-on-{}-{}",
                coin_full(coin),
                month,
                day
            ))
        }
    }
}

fn ny_noon_for_date(date: chrono::NaiveDate) -> Option<chrono::DateTime<chrono_tz::Tz>> {
    New_York
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 12, 0, 0)
        .single()
}

fn utc_from_ts(ts: i64) -> Result<DateTime<Utc>, SlugError> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .ok_or(SlugError::InvalidTimestamp(ts))
}

fn coin_short(coin: Coin) -> &'static str {
    match coin {
        Coin::Btc => "btc",
        Coin::Eth => "eth",
        Coin::Sol => "sol",
        Coin::Xrp => "xrp",
    }
}

fn coin_full(coin: Coin) -> &'static str {
    match coin {
        Coin::Btc => "bitcoin",
        Coin::Eth => "ethereum",
        Coin::Sol => "solana",
        Coin::Xrp => "xrp",
    }
}

fn align_4h_start_ts_et(start_ts_utc: i64) -> Result<i64, SlugError> {
    let ny = utc_from_ts(start_ts_utc)?.with_timezone(&New_York);
    let date = ny.date_naive();
    let block_hour = (ny.hour() / 4) * 4;
    let aligned = New_York
        .with_ymd_and_hms(date.year(), date.month(), date.day(), block_hour, 0, 0)
        .single()
        .ok_or(SlugError::InvalidTimestamp(start_ts_utc))?;
    Ok(aligned.with_timezone(&Utc).timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn cfg(offset_min: i32) -> SlugConfig {
        SlugConfig {
            discovery_offset_4h_min: offset_min,
        }
    }

    #[test]
    fn formats_5m_for_all_coins() {
        let ts = 1_771_449_000;
        let cases = [
            (Coin::Btc, "btc-updown-5m-1771449000"),
            (Coin::Eth, "eth-updown-5m-1771449000"),
            (Coin::Sol, "sol-updown-5m-1771449000"),
            (Coin::Xrp, "xrp-updown-5m-1771449000"),
        ];

        for (coin, expected) in cases {
            let actual = build_slug(coin, Duration::M5, ts, SlugConfig::default()).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn formats_15m_for_all_coins() {
        let ts = 1_771_448_400;
        let cases = [
            (Coin::Btc, "btc-updown-15m-1771448400"),
            (Coin::Eth, "eth-updown-15m-1771448400"),
            (Coin::Sol, "sol-updown-15m-1771448400"),
            (Coin::Xrp, "xrp-updown-15m-1771448400"),
        ];

        for (coin, expected) in cases {
            let actual = build_slug(coin, Duration::M15, ts, SlugConfig::default()).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn formats_4h_for_all_coins_aligned_to_et_boundaries() {
        // 2025-01-01 03:10:00 UTC = 2024-12-31 22:10 ET -> aligned to 20:00 ET.
        let ts = 1_735_702_200;
        let cases = [
            (Coin::Btc, "btc-updown-4h-1735693200"),
            (Coin::Eth, "eth-updown-4h-1735693200"),
            (Coin::Sol, "sol-updown-4h-1735693200"),
            (Coin::Xrp, "xrp-updown-4h-1735693200"),
        ];

        for (coin, expected) in cases {
            let actual = build_slug(coin, Duration::H4, ts, SlugConfig::default()).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn formats_1h_for_all_coins_normal_day() {
        // 2025-01-15 20:00:00 UTC -> 2025-01-15 3pm ET
        let ts = 1_736_971_200;
        let cases = [
            (Coin::Btc, "bitcoin-up-or-down-january-15-3pm-et"),
            (Coin::Eth, "ethereum-up-or-down-january-15-3pm-et"),
            (Coin::Sol, "solana-up-or-down-january-15-3pm-et"),
            (Coin::Xrp, "xrp-up-or-down-january-15-3pm-et"),
        ];

        for (coin, expected) in cases {
            let actual = build_slug(coin, Duration::H1, ts, SlugConfig::default()).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn formats_1h_for_est_and_edt_periods() {
        // EST: Jan 10, 2025 13:00 UTC => 8am ET
        let est = build_slug(
            Coin::Btc,
            Duration::H1,
            1_736_514_000,
            SlugConfig::default(),
        )
        .unwrap();
        assert_eq!(est, "bitcoin-up-or-down-january-10-8am-et");

        // EDT: Jul 10, 2025 13:00 UTC => 9am ET
        let edt = build_slug(
            Coin::Btc,
            Duration::H1,
            1_752_152_400,
            SlugConfig::default(),
        )
        .unwrap();
        assert_eq!(edt, "bitcoin-up-or-down-july-10-9am-et");
    }

    #[test]
    fn formats_1d_for_all_coins_using_resolution_date() {
        // 2026-02-25 17:00 UTC = 2026-02-25 12:00 ET (daily market start)
        // Resolution is 2026-02-26 12:00 ET -> slug date is february-26.
        let ts = 1_772_040_400;
        let cases = [
            (Coin::Btc, "bitcoin-up-or-down-on-february-26"),
            (Coin::Eth, "ethereum-up-or-down-on-february-26"),
            (Coin::Sol, "solana-up-or-down-on-february-26"),
            (Coin::Xrp, "xrp-up-or-down-on-february-26"),
        ];

        for (coin, expected) in cases {
            let actual = build_slug(coin, Duration::D1, ts, SlugConfig::default()).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn invalid_duration_and_coin_errors_are_explicit() {
        assert_eq!(
            parse_duration("2h").unwrap_err(),
            SlugError::InvalidDuration("2h".to_string())
        );
        assert_eq!(
            parse_coin("DOGE").unwrap_err(),
            SlugError::UnsupportedCoin("DOGE".to_string())
        );
    }

    #[test]
    fn boundary_time_exactly_on_interval_start_is_stable() {
        // Exactly on 4h ET boundary (2025-01-01 01:00 UTC = 20:00 ET).
        let exact_boundary = 1_735_693_200;
        let slug = build_slug(
            Coin::Eth,
            Duration::H4,
            exact_boundary,
            SlugConfig::default(),
        )
        .unwrap();
        assert_eq!(slug, "eth-updown-4h-1735693200");
    }

    #[test]
    fn deterministic_sweep_and_pattern_checks() {
        let re_5m = Regex::new(r"^[a-z]+-updown-5m-\d+$").unwrap();
        let re_15m = Regex::new(r"^[a-z]+-updown-15m-\d+$").unwrap();
        let re_4h = Regex::new(r"^[a-z]+-updown-4h-\d+$").unwrap();
        let re_1h = Regex::new(r"^[a-z]+-up-or-down-[a-z]+-\d{1,2}-\d{1,2}(am|pm)-et$").unwrap();
        let re_1d = Regex::new(r"^[a-z]+-up-or-down-on-[a-z]+-\d{1,2}$").unwrap();

        for ts in (1_735_689_600..1_735_776_000).step_by(61) {
            let a = build_slug(Coin::Btc, Duration::M5, ts, cfg(15)).unwrap();
            let b = build_slug(Coin::Btc, Duration::M5, ts, cfg(15)).unwrap();
            assert_eq!(a, b);
            assert!(re_5m.is_match(&a));

            let a = build_slug(Coin::Eth, Duration::M15, ts, cfg(15)).unwrap();
            let b = build_slug(Coin::Eth, Duration::M15, ts, cfg(15)).unwrap();
            assert_eq!(a, b);
            assert!(re_15m.is_match(&a));

            let a = build_slug(Coin::Sol, Duration::H4, ts, cfg(15)).unwrap();
            let b = build_slug(Coin::Sol, Duration::H4, ts, cfg(15)).unwrap();
            assert_eq!(a, b);
            assert!(re_4h.is_match(&a));

            let a = build_slug(Coin::Xrp, Duration::H1, ts, cfg(15)).unwrap();
            let b = build_slug(Coin::Xrp, Duration::H1, ts, cfg(15)).unwrap();
            assert_eq!(a, b);
            assert!(re_1h.is_match(&a));

            let a = build_slug(Coin::Btc, Duration::D1, ts, cfg(15)).unwrap();
            let b = build_slug(Coin::Btc, Duration::D1, ts, cfg(15)).unwrap();
            assert_eq!(a, b);
            assert!(re_1d.is_match(&a));
        }
    }
}

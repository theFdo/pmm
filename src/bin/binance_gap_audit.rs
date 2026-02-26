use std::collections::HashMap;

use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use pmm::{load_1s_klines, BinanceSymbol, HistoricalKlinesConfig, KlineLoadRequest};

#[derive(Default, Debug, Clone, Copy)]
struct Totals {
    expected: u64,
    actual: u64,
    missing: u64,
    duplicates_removed: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start_date = NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid start date");
    let today_utc = Utc::now().date_naive();
    let end_date_exclusive = today_utc; // latest completed UTC day

    let start_ts = day_start_ts_ms(start_date);
    let end_ts = day_start_ts_ms(end_date_exclusive);

    if end_ts <= start_ts {
        return Err(format!(
            "invalid audit range: start={} end={} (exclusive)",
            start_date, end_date_exclusive
        )
        .into());
    }

    let cfg = HistoricalKlinesConfig {
        data_root: std::path::PathBuf::from("data/binance"),
        verify_checksum: true,
        ..HistoricalKlinesConfig::default()
    };

    println!(
        "Running Binance 1s combined audit for BTCUSDT/ETHUSDT/SOLUSDT/XRPUSDT from {} 00:00:00 UTC to {} 00:00:00 UTC (exclusive)",
        start_date,
        end_date_exclusive
    );

    let symbols = [
        BinanceSymbol::BtcUsdt,
        BinanceSymbol::EthUsdt,
        BinanceSymbol::SolUsdt,
        BinanceSymbol::XrpUsdt,
    ];

    let mut totals: HashMap<BinanceSymbol, Totals> = symbols
        .iter()
        .copied()
        .map(|symbol| (symbol, Totals::default()))
        .collect();

    let mut gap_ranges_by_symbol: HashMap<BinanceSymbol, Vec<(i64, i64)>> = symbols
        .iter()
        .copied()
        .map(|symbol| (symbol, Vec::new()))
        .collect();

    // Pass 1: monthly windows, all symbols together.
    let mut cursor = start_ts;
    while cursor < end_ts {
        let window_start_date = Utc
            .timestamp_millis_opt(cursor)
            .single()
            .expect("valid cursor timestamp")
            .date_naive();
        let next_month = if window_start_date.month() == 12 {
            NaiveDate::from_ymd_opt(window_start_date.year() + 1, 1, 1).expect("valid next month")
        } else {
            NaiveDate::from_ymd_opt(window_start_date.year(), window_start_date.month() + 1, 1)
                .expect("valid next month")
        };
        let window_end = std::cmp::min(day_start_ts_ms(next_month), end_ts);
        let window_end_date = Utc
            .timestamp_millis_opt(window_end)
            .single()
            .expect("valid window end timestamp")
            .date_naive();

        println!("\nWindow {} -> {}", window_start_date, window_end_date);

        for symbol in symbols {
            let req = KlineLoadRequest {
                symbol,
                start_ts_ms_utc: cursor,
                end_ts_ms_utc_exclusive: window_end,
            };
            let loaded = load_1s_klines(&req, &cfg)?;

            let t = totals
                .get_mut(&symbol)
                .expect("totals map should contain symbol");
            t.expected += loaded.coverage.expected_points;
            t.actual += loaded.coverage.actual_points;
            t.missing += loaded.coverage.missing_points;
            t.duplicates_removed += loaded.coverage.duplicate_points_removed;

            println!(
                "  {} | expected={} actual={} missing={} dupes_removed={}",
                symbol.as_str(),
                loaded.coverage.expected_points,
                loaded.coverage.actual_points,
                loaded.coverage.missing_points,
                loaded.coverage.duplicate_points_removed
            );

            if loaded.coverage.missing_points > 0 {
                let symbol_ranges = gap_ranges_by_symbol
                    .get_mut(&symbol)
                    .expect("gap map should contain symbol");
                for (gap_start, gap_end) in loaded.coverage.gap_ranges {
                    symbol_ranges.push((gap_start, gap_end));
                }
            }
        }

        cursor = window_end;
    }

    let mut initial_missing_total = 0u64;
    for symbol in symbols {
        let t = totals
            .get(&symbol)
            .expect("totals map should contain symbol");
        initial_missing_total += t.missing;
        println!(
            "\nINITIAL TOTAL {} | expected={} actual={} missing={} dupes_removed={}",
            symbol.as_str(),
            t.expected,
            t.actual,
            t.missing,
            t.duplicates_removed
        );
    }

    // Pass 2: refill only missing ranges (daily planner path), then recheck those exact ranges.
    let mut remaining_missing = 0u64;
    if initial_missing_total > 0 {
        println!("\nRefill pass: downloading only missing ranges per symbol...");
        for symbol in symbols {
            let ranges = gap_ranges_by_symbol
                .get(&symbol)
                .cloned()
                .unwrap_or_default();
            if ranges.is_empty() {
                continue;
            }

            for (gap_start, gap_end_inclusive) in ranges {
                let req = KlineLoadRequest {
                    symbol,
                    start_ts_ms_utc: gap_start,
                    end_ts_ms_utc_exclusive: gap_end_inclusive.saturating_add(1_000),
                };
                let loaded = load_1s_klines(&req, &cfg)?;
                if loaded.coverage.missing_points > 0 {
                    remaining_missing += loaded.coverage.missing_points;
                    println!(
                        "  refill unresolved {} | {} -> {} missing={}",
                        symbol.as_str(),
                        gap_start,
                        gap_end_inclusive,
                        loaded.coverage.missing_points
                    );
                }
            }
        }
    }

    if initial_missing_total == 0 {
        println!("\nRESULT: no gaps detected across all symbols in audited range.");
        return Ok(());
    }

    if remaining_missing == 0 {
        println!(
            "\nRESULT: initial gaps were fully refillable from historical archives (remaining_missing=0)."
        );
        Ok(())
    } else {
        Err(format!(
            "gaps remain after refill pass: initial_missing={} remaining_missing={}",
            initial_missing_total, remaining_missing
        )
        .into())
    }
}

fn day_start_ts_ms(date: NaiveDate) -> i64 {
    Utc.with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .expect("valid day start")
        .timestamp_millis()
}

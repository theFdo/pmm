use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::TimeZone;
use pmm::{
    load_1s_klines, plan_required_archives, sync_archives, ArchiveKind, BinanceSymbol,
    HistoricalKlinesConfig, KlineLoadRequest, LocalArchiveSource,
};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

fn ts_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
    chrono::Utc
        .with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("valid UTC timestamp expected")
        .timestamp_millis()
}

fn write_zip(path: &Path, csv_body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be creatable");
    }

    let file = fs::File::create(path).expect("zip file should be created");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("data.csv", SimpleFileOptions::default())
        .expect("zip entry should be created");
    zip.write_all(csv_body.as_bytes())
        .expect("zip data should be written");
    zip.finish().expect("zip should finalize");
}

fn sample_path_for(symbol: BinanceSymbol) -> &'static str {
    match symbol {
        BinanceSymbol::BtcUsdt => "tests/fixtures/binance/BTCUSDT_1s_sample.csv",
        BinanceSymbol::EthUsdt => "tests/fixtures/binance/ETHUSDT_1s_sample.csv",
        BinanceSymbol::SolUsdt => "tests/fixtures/binance/SOLUSDT_1s_sample.csv",
        BinanceSymbol::XrpUsdt => "tests/fixtures/binance/XRPUSDT_1s_sample.csv",
    }
}

#[test]
fn fixture_load_test_per_coin_pair() {
    let start = ts_ms(2024, 1, 1, 0, 0, 0);
    let end = start + 3_000;

    for symbol in [
        BinanceSymbol::BtcUsdt,
        BinanceSymbol::EthUsdt,
        BinanceSymbol::SolUsdt,
        BinanceSymbol::XrpUsdt,
    ] {
        let temp = tempdir().expect("temp dir should be created");
        let cfg = HistoricalKlinesConfig {
            data_root: temp.path().to_path_buf(),
            verify_checksum: false,
            ..HistoricalKlinesConfig::default()
        };
        let req = KlineLoadRequest {
            symbol,
            start_ts_ms_utc: start,
            end_ts_ms_utc_exclusive: end,
        };

        let archives = plan_required_archives(&req);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].kind, ArchiveKind::Daily);

        let csv = fs::read_to_string(sample_path_for(symbol)).expect("fixture should exist");
        let archive_path = cfg.data_root.join(&archives[0].relative_path);
        write_zip(&archive_path, &csv);

        let loaded = load_1s_klines(&req, &cfg).expect("fixture load should succeed");
        assert_eq!(loaded.symbol, symbol);
        assert_eq!(loaded.rows.len(), 3);
        assert_eq!(loaded.coverage.expected_points, 3);
        assert_eq!(loaded.coverage.actual_points, 3);
        assert_eq!(loaded.coverage.missing_points, 0);
        assert_eq!(loaded.coverage.duplicate_points_removed, 0);
    }
}

#[test]
fn stitch_monthly_and_daily_with_overlap_dedupes_and_sorts() {
    let req = KlineLoadRequest {
        symbol: BinanceSymbol::BtcUsdt,
        start_ts_ms_utc: ts_ms(2024, 2, 1, 0, 0, 0),
        end_ts_ms_utc_exclusive: ts_ms(2024, 3, 2, 0, 0, 0),
    };
    let temp = tempdir().expect("temp dir should be created");
    let cfg = HistoricalKlinesConfig {
        data_root: temp.path().to_path_buf(),
        verify_checksum: false,
        ..HistoricalKlinesConfig::default()
    };

    let feb_29_235958 = ts_ms(2024, 2, 29, 23, 59, 58);
    let feb_29_235959 = ts_ms(2024, 2, 29, 23, 59, 59);
    let mar_01_000000 = ts_ms(2024, 3, 1, 0, 0, 0);
    let mar_01_000001 = ts_ms(2024, 3, 1, 0, 0, 1);

    let archives = plan_required_archives(&req);
    assert!(archives.iter().any(|a| a.kind == ArchiveKind::Monthly));
    assert!(archives.iter().any(|a| a.kind == ArchiveKind::Daily));

    for archive in &archives {
        let csv = match archive.kind {
            ArchiveKind::Monthly => format!(
                "{feb_29_235958},100,100,100,100,1,{end1},100,1,1,1,0\n{feb_29_235959},101,101,101,101,1,{end2},101,1,1,1,0\n{mar_01_000000},102,102,102,102,1,{end3},102,1,1,1,0\n",
                end1 = feb_29_235958 + 999,
                end2 = feb_29_235959 + 999,
                end3 = mar_01_000000 + 999,
            ),
            ArchiveKind::Daily => format!(
                "{mar_01_000000},200,200,200,200,1,{end3},200,1,1,1,0\n{mar_01_000001},201,201,201,201,1,{end4},201,1,1,1,0\n",
                end3 = mar_01_000000 + 999,
                end4 = mar_01_000001 + 999,
            ),
        };

        let path = cfg.data_root.join(&archive.relative_path);
        write_zip(&path, &csv);
    }

    let loaded = load_1s_klines(&req, &cfg).expect("load should succeed");
    assert_eq!(loaded.coverage.actual_points, 4);
    assert_eq!(loaded.coverage.duplicate_points_removed, 1);

    let times: Vec<i64> = loaded.rows.iter().map(|row| row.open_time_ms).collect();
    assert_eq!(
        times,
        vec![feb_29_235958, feb_29_235959, mar_01_000000, mar_01_000001,]
    );
}

#[test]
fn coverage_report_detects_missing_second_gap() {
    let start = ts_ms(2024, 1, 1, 0, 0, 0);
    let req = KlineLoadRequest {
        symbol: BinanceSymbol::EthUsdt,
        start_ts_ms_utc: start,
        end_ts_ms_utc_exclusive: start + 3_000,
    };

    let temp = tempdir().expect("temp dir should be created");
    let cfg = HistoricalKlinesConfig {
        data_root: temp.path().to_path_buf(),
        verify_checksum: false,
        ..HistoricalKlinesConfig::default()
    };

    let archives = plan_required_archives(&req);
    assert_eq!(archives.len(), 1);
    let csv = format!(
        "{start},1,1,1,1,1,{end0},1,1,1,1,0\n{start2},2,2,2,2,1,{end2},2,1,1,1,0\n",
        end0 = start + 999,
        start2 = start + 2_000,
        end2 = start + 2_999,
    );
    write_zip(&cfg.data_root.join(&archives[0].relative_path), &csv);

    let loaded = load_1s_klines(&req, &cfg).expect("load should succeed");
    assert_eq!(loaded.coverage.expected_points, 3);
    assert_eq!(loaded.coverage.actual_points, 2);
    assert_eq!(loaded.coverage.missing_points, 1);
    assert_eq!(
        loaded.coverage.gap_ranges,
        vec![(start + 1_000, start + 1_000)]
    );
}

#[test]
fn sync_archives_uses_cached_files_when_checksum_verification_disabled() {
    let start = ts_ms(2024, 1, 1, 0, 0, 0);
    let req = KlineLoadRequest {
        symbol: BinanceSymbol::SolUsdt,
        start_ts_ms_utc: start,
        end_ts_ms_utc_exclusive: start + 3_000,
    };

    let temp = tempdir().expect("temp dir should be created");
    let cfg = HistoricalKlinesConfig {
        data_root: temp.path().to_path_buf(),
        verify_checksum: false,
        ..HistoricalKlinesConfig::default()
    };

    let archives = plan_required_archives(&req);
    assert_eq!(archives.len(), 1);
    let csv =
        fs::read_to_string(sample_path_for(BinanceSymbol::SolUsdt)).expect("fixture should exist");
    write_zip(&cfg.data_root.join(&archives[0].relative_path), &csv);

    let synced = sync_archives(&req, &cfg).expect("sync should succeed");
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].source, LocalArchiveSource::Cached);
}

#[cfg(feature = "live-binance-tests")]
#[test]
#[ignore = "requires external network access"]
fn live_binance_download_smoke() {
    let req = KlineLoadRequest {
        symbol: BinanceSymbol::BtcUsdt,
        start_ts_ms_utc: ts_ms(2024, 1, 1, 0, 0, 0),
        end_ts_ms_utc_exclusive: ts_ms(2024, 1, 2, 0, 0, 0),
    };

    let temp = tempdir().expect("temp dir should be created");
    let cfg = HistoricalKlinesConfig {
        data_root: temp.path().to_path_buf(),
        verify_checksum: true,
        ..HistoricalKlinesConfig::default()
    };

    let loaded = load_1s_klines(&req, &cfg).expect("live load should succeed");
    assert!(!loaded.rows.is_empty());
}

use pmm::{
    assert_schema_compatible, build_feature_schema, horizon_conditioning, transform_store_range,
    transform_store_range_for_runtime_cold_start, transform_store_range_for_training, FeatureError,
    FeatureTransformConfig, FeatureTransformRequest, GapPolicy, FEATURE_SCHEMA_VERSION,
};
use rusqlite::{params, Connection};
use tempfile::NamedTempFile;

const START_TS_MS: i64 = 1_735_689_600_000; // 2025-01-01T00:00:00Z
const STEP_MS: i64 = 1_000;

#[test]
fn schema_order_and_fingerprint_are_deterministic() {
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![5, 15],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::Strict,
        schema_version: FEATURE_SCHEMA_VERSION,
    };

    let schema_a = build_feature_schema(&cfg);
    let schema_b = build_feature_schema(&cfg);

    assert_eq!(schema_a.version, FEATURE_SCHEMA_VERSION);
    assert_eq!(schema_a.columns.len(), 38);
    assert_eq!(schema_a.columns[0].name, "btc_ret_1s");
    assert_eq!(schema_a.columns[1].name, "btc_ret_5s");
    assert_eq!(schema_a.columns[2].name, "btc_ret_15s");
    assert_eq!(schema_a.columns[3].name, "btc_range_5s");
    assert_eq!(schema_a.columns[4].name, "btc_range_15s");
    assert_eq!(schema_a.columns[36].name, "tow_sin");
    assert_eq!(schema_a.columns[37].name, "tow_cos");
    assert_eq!(schema_a, schema_b);
}

#[test]
fn horizon_conditioning_matches_expected_formula() {
    let hc = horizon_conditioning(3_600, 86_400);
    let expected_log = (1.0_f64 + 3_600.0).ln() / (1.0_f64 + 86_400.0).ln();
    let expected_sqrt = 3_600.0_f64.sqrt() / 86_400.0_f64.sqrt();
    assert!((hc.log_horizon_norm - expected_log).abs() < 1e-12);
    assert!((hc.sqrt_horizon_norm - expected_sqrt).abs() < 1e-12);
}

#[test]
fn transform_is_deterministic_and_emits_expected_math() {
    let tmp = seed_store(START_TS_MS, 10, None, &[]);
    let req = FeatureTransformRequest {
        start_ts_ms_utc: START_TS_MS,
        end_ts_ms_utc_exclusive: START_TS_MS + 10 * STEP_MS,
    };
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![2, 3],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::Strict,
        schema_version: FEATURE_SCHEMA_VERSION,
    };

    let out_a = transform_store_range(tmp.path(), &req, &cfg).expect("first transform succeeds");
    let out_b = transform_store_range(tmp.path(), &req, &cfg).expect("second transform succeeds");

    assert_eq!(out_a.0, out_b.0);
    assert_eq!(out_a.1, out_b.1);
    assert_eq!(out_a.2, out_b.2);

    let schema = out_a.0;
    let rows = out_a.1;
    let report = out_a.2;

    assert_eq!(rows.len(), 7);
    assert_eq!(report.input_points, 10);
    assert_eq!(report.output_points, 7);
    assert_eq!(report.skipped_points, 0);
    assert!(report.gap_ranges.is_empty());
    assert_eq!(rows[0].ts_ms_utc, START_TS_MS + 3 * STEP_MS);
    assert_eq!(rows[0].values.len(), schema.columns.len());

    let b = btc_base_close();
    let t = 3.0_f64;
    let expected_ret_1s = ((b + t) / (b + t - 1.0)).ln();
    let expected_ret_2s = ((b + t) / (b + t - 2.0)).ln();
    let expected_ret_3s = ((b + t) / (b + t - 3.0)).ln();
    let expected_range_2s = ((b + t + 0.5) / (b + t - 1.0 - 0.5)).ln();
    let expected_range_3s = ((b + t + 0.5) / (b + t - 2.0 - 0.5)).ln();

    let r1 = ((b + 1.0) / b).ln();
    let r2 = ((b + 2.0) / (b + 1.0)).ln();
    let r3 = ((b + 3.0) / (b + 2.0)).ln();
    let expected_vol_2s = stddev(&[r2, r3]);
    let expected_vol_3s = stddev(&[r1, r2, r3]);

    let q1 = btc_quote_vol(1.0);
    let q2 = btc_quote_vol(2.0);
    let q3 = btc_quote_vol(3.0);
    let expected_qv_2s = (1.0 + q2 + q3).ln();
    let expected_qv_3s = (1.0 + q1 + q2 + q3).ln();

    assert_close(
        rows[0].values[column_index(&schema, "btc_ret_1s")],
        expected_ret_1s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_ret_2s")],
        expected_ret_2s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_ret_3s")],
        expected_ret_3s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_range_2s")],
        expected_range_2s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_range_3s")],
        expected_range_3s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_vol_2s")],
        expected_vol_2s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_vol_3s")],
        expected_vol_3s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_quote_vol_2s")],
        expected_qv_2s,
    );
    assert_close(
        rows[0].values[column_index(&schema, "btc_quote_vol_3s")],
        expected_qv_3s,
    );
}

#[test]
fn strict_policy_fails_on_incomplete_frame() {
    let tmp = seed_store(START_TS_MS, 6, Some((3, 2)), &[]);
    let req = FeatureTransformRequest {
        start_ts_ms_utc: START_TS_MS,
        end_ts_ms_utc_exclusive: START_TS_MS + 6 * STEP_MS,
    };
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![2],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::Strict,
        schema_version: FEATURE_SCHEMA_VERSION,
    };

    let err = transform_store_range(tmp.path(), &req, &cfg).expect_err("must fail");
    match err {
        FeatureError::IncompleteFrame {
            ts_ms_utc,
            missing_symbols,
        } => {
            assert_eq!(ts_ms_utc, START_TS_MS + 3 * STEP_MS);
            assert_eq!(missing_symbols, vec!["ETH"]);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn report_and_skip_keeps_running_and_reports_gap() {
    let tmp = seed_store(START_TS_MS, 6, Some((3, 2)), &[]);
    let req = FeatureTransformRequest {
        start_ts_ms_utc: START_TS_MS,
        end_ts_ms_utc_exclusive: START_TS_MS + 6 * STEP_MS,
    };
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![2],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::ReportAndSkip,
        schema_version: FEATURE_SCHEMA_VERSION,
    };

    let (_schema, rows, report) =
        transform_store_range(tmp.path(), &req, &cfg).expect("transform succeeds");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ts_ms_utc, START_TS_MS + 2 * STEP_MS);
    assert!(report.skipped_points >= 1);
    assert_eq!(
        report.gap_ranges[0],
        (START_TS_MS + 3 * STEP_MS, START_TS_MS + 4 * STEP_MS)
    );
    assert!(report.first_error.is_some());
}

#[test]
fn strict_policy_fails_on_missing_timestamp_continuity_gap() {
    let tmp = seed_store(START_TS_MS, 7, None, &[4]);
    let req = FeatureTransformRequest {
        start_ts_ms_utc: START_TS_MS,
        end_ts_ms_utc_exclusive: START_TS_MS + 7 * STEP_MS,
    };
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![2],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::Strict,
        schema_version: FEATURE_SCHEMA_VERSION,
    };

    let err = transform_store_range(tmp.path(), &req, &cfg).expect_err("must fail");
    match err {
        FeatureError::ContinuityGap {
            expected_next_ts_ms_utc,
            actual_ts_ms_utc,
            missing_points,
        } => {
            assert_eq!(expected_next_ts_ms_utc, START_TS_MS + 4 * STEP_MS);
            assert_eq!(actual_ts_ms_utc, START_TS_MS + 5 * STEP_MS);
            assert_eq!(missing_points, 1);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn schema_compatibility_check_matches_version_and_fingerprint() {
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![2],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::Strict,
        schema_version: FEATURE_SCHEMA_VERSION,
    };
    let schema = build_feature_schema(&cfg);

    assert_schema_compatible(FEATURE_SCHEMA_VERSION, &schema.fingerprint, &schema)
        .expect("compatibility should pass");

    let err = assert_schema_compatible(FEATURE_SCHEMA_VERSION + 1, &schema.fingerprint, &schema)
        .expect_err("version mismatch expected");
    assert!(matches!(err, FeatureError::SchemaVersionMismatch { .. }));

    let err = assert_schema_compatible(FEATURE_SCHEMA_VERSION, "not-real", &schema)
        .expect_err("fingerprint mismatch expected");
    assert!(matches!(
        err,
        FeatureError::SchemaFingerprintMismatch { .. }
    ));
}

#[test]
fn training_and_runtime_wrappers_share_same_transform_output() {
    let tmp = seed_store(START_TS_MS, 10, None, &[]);
    let req = FeatureTransformRequest {
        start_ts_ms_utc: START_TS_MS,
        end_ts_ms_utc_exclusive: START_TS_MS + 10 * STEP_MS,
    };
    let cfg = FeatureTransformConfig {
        windows_seconds: vec![2],
        max_duration_seconds: 86_400,
        gap_policy: GapPolicy::Strict,
        schema_version: FEATURE_SCHEMA_VERSION,
    };

    let training = transform_store_range_for_training(tmp.path(), &req, &cfg).expect("training");
    let runtime =
        transform_store_range_for_runtime_cold_start(tmp.path(), &req, &cfg).expect("runtime");
    assert_eq!(training, runtime);
}

fn seed_store(
    start_ts_ms: i64,
    points: usize,
    missing_symbol: Option<(usize, i64)>,
    skipped_timestamps: &[usize],
) -> NamedTempFile {
    let file = NamedTempFile::new().expect("temp sqlite file");
    let conn = Connection::open(file.path()).expect("open sqlite");
    create_kline_schema(&conn);

    for t_idx in 0..points {
        if skipped_timestamps.contains(&t_idx) {
            continue;
        }
        let ts_ms = start_ts_ms + t_idx as i64 * STEP_MS;
        for symbol_id in 1_i64..=4_i64 {
            if missing_symbol == Some((t_idx, symbol_id)) {
                continue;
            }
            let close = base_close(symbol_id) + t_idx as f64;
            let high = close + 0.5;
            let low = close - 0.5;
            let quote_vol = 1_000.0 + symbol_id as f64 * 10.0 + t_idx as f64;
            insert_kline_row(&conn, symbol_id, ts_ms, high, low, close, quote_vol);
        }
    }

    file
}

fn create_kline_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE klines_1s (
            symbol_id INTEGER NOT NULL,
            open_time_ms INTEGER NOT NULL,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            volume REAL NOT NULL,
            close_time_ms INTEGER NOT NULL,
            quote_asset_volume REAL NOT NULL,
            trade_count INTEGER NOT NULL,
            taker_buy_base_volume REAL NOT NULL,
            taker_buy_quote_volume REAL NOT NULL,
            PRIMARY KEY(symbol_id, open_time_ms)
        ) WITHOUT ROWID;
        ",
    )
    .expect("create schema");
}

fn insert_kline_row(
    conn: &Connection,
    symbol_id: i64,
    open_time_ms: i64,
    high: f64,
    low: f64,
    close: f64,
    quote_asset_volume: f64,
) {
    conn.execute(
        "
        INSERT INTO klines_1s (
            symbol_id,
            open_time_ms,
            open,
            high,
            low,
            close,
            volume,
            close_time_ms,
            quote_asset_volume,
            trade_count,
            taker_buy_base_volume,
            taker_buy_quote_volume
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ",
        params![
            symbol_id,
            open_time_ms,
            close,
            high,
            low,
            close,
            50.0_f64,
            open_time_ms + 999,
            quote_asset_volume,
            100_u64,
            10.0_f64,
            20.0_f64
        ],
    )
    .expect("insert row");
}

fn base_close(symbol_id: i64) -> f64 {
    1_000.0 + 100.0 * (symbol_id as f64 - 1.0)
}

fn btc_base_close() -> f64 {
    base_close(1)
}

fn btc_quote_vol(t_idx: f64) -> f64 {
    1_000.0 + 10.0 + t_idx
}

fn stddev(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|v| {
            let d = *v - mean;
            d * d
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "actual={actual} expected={expected}"
    );
}

fn column_index(schema: &pmm::FeatureSchema, name: &str) -> usize {
    schema
        .columns
        .iter()
        .position(|column| column.name == name)
        .expect("column must exist")
}

//! Step 9 shared bars-to-features transform.

use std::collections::{HashSet, VecDeque};
use std::f64::consts::PI;
use std::path::Path;

use chrono::{Datelike, TimeZone, Timelike, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{info, warn};

const STEP_MS: i64 = 1_000;
const SYMBOL_COUNT: usize = 4;
const WEEK_SECONDS: f64 = 7.0 * 24.0 * 60.0 * 60.0;
const MAX_REPORTED_GAP_RANGES: usize = 256;

pub const FEATURE_SCHEMA_VERSION: u32 = 1;

const SYMBOL_CODES: [&str; SYMBOL_COUNT] = ["btc", "eth", "sol", "xrp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapPolicy {
    Strict,
    ReportAndSkip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureDType {
    F64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureColumn {
    pub name: String,
    pub dtype: FeatureDType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSchema {
    pub version: u32,
    pub fingerprint: String,
    pub columns: Vec<FeatureColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureRow {
    pub ts_ms_utc: i64,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureTransformRequest {
    pub start_ts_ms_utc: i64,
    pub end_ts_ms_utc_exclusive: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureTransformReport {
    pub input_points: u64,
    pub output_points: u64,
    pub skipped_points: u64,
    pub gap_ranges: Vec<(i64, i64)>,
    pub first_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureTransformConfig {
    pub windows_seconds: Vec<u32>,
    pub max_duration_seconds: u32,
    pub gap_policy: GapPolicy,
    pub schema_version: u32,
}

impl Default for FeatureTransformConfig {
    fn default() -> Self {
        Self {
            windows_seconds: vec![5, 15, 60],
            max_duration_seconds: 86_400,
            gap_policy: GapPolicy::Strict,
            schema_version: FEATURE_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizonConditioning {
    pub log_horizon_norm: f64,
    pub sqrt_horizon_norm: f64,
}

#[derive(Debug, Error)]
pub enum FeatureError {
    #[error("invalid feature transform request: {0}")]
    InvalidRequest(String),
    #[error("invalid feature transform config: {0}")]
    InvalidConfig(String),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid UTC timestamp: {0}")]
    InvalidTimestamp(i64),
    #[error("invalid symbol_id {symbol_id} at {ts_ms_utc}")]
    UnknownSymbolId { symbol_id: i64, ts_ms_utc: i64 },
    #[error("duplicate symbol_id {symbol_id} at {ts_ms_utc}")]
    DuplicateSymbolInFrame { symbol_id: i64, ts_ms_utc: i64 },
    #[error("incomplete timestamp frame at {ts_ms_utc}; missing symbols: {missing_symbols:?}")]
    IncompleteFrame {
        ts_ms_utc: i64,
        missing_symbols: Vec<String>,
    },
    #[error(
        "continuity gap detected from {expected_next_ts_ms_utc} to {actual_ts_ms_utc} ({missing_points} missing points)"
    )]
    ContinuityGap {
        expected_next_ts_ms_utc: i64,
        actual_ts_ms_utc: i64,
        missing_points: u64,
    },
    #[error("schema version mismatch: expected {expected}, got {actual}")]
    SchemaVersionMismatch { expected: u32, actual: u32 },
    #[error("schema fingerprint mismatch: expected {expected}, got {actual}")]
    SchemaFingerprintMismatch { expected: String, actual: String },
}

#[derive(Debug, Clone, Copy)]
struct KlinePoint {
    high: f64,
    low: f64,
    close: f64,
    quote_asset_volume: f64,
}

#[derive(Debug, Clone)]
struct Frame {
    ts_ms_utc: i64,
    points: [Option<KlinePoint>; SYMBOL_COUNT],
}

impl Frame {
    fn new(ts_ms_utc: i64) -> Self {
        Self {
            ts_ms_utc,
            points: [None; SYMBOL_COUNT],
        }
    }
}

#[derive(Debug, Clone)]
struct SymbolRolling {
    closes: VecDeque<f64>,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    quote_volumes: VecDeque<f64>,
    ret_1s: VecDeque<f64>,
    max_window: usize,
}

impl SymbolRolling {
    fn new(max_window: usize) -> Self {
        Self {
            closes: VecDeque::new(),
            highs: VecDeque::new(),
            lows: VecDeque::new(),
            quote_volumes: VecDeque::new(),
            ret_1s: VecDeque::new(),
            max_window,
        }
    }

    fn reset(&mut self) {
        self.closes.clear();
        self.highs.clear();
        self.lows.clear();
        self.quote_volumes.clear();
        self.ret_1s.clear();
    }

    fn push(&mut self, point: KlinePoint) {
        if let Some(prev_close) = self.closes.back().copied() {
            self.ret_1s.push_back((point.close / prev_close).ln());
            while self.ret_1s.len() > self.max_window {
                self.ret_1s.pop_front();
            }
        }

        self.closes.push_back(point.close);
        self.highs.push_back(point.high);
        self.lows.push_back(point.low);
        self.quote_volumes.push_back(point.quote_asset_volume);

        while self.closes.len() > self.max_window + 1 {
            self.closes.pop_front();
            self.highs.pop_front();
            self.lows.pop_front();
            self.quote_volumes.pop_front();
        }
    }

    fn ret_1s(&self) -> Option<f64> {
        self.ret_1s.back().copied()
    }

    fn ret_w(&self, w: usize) -> Option<f64> {
        if self.closes.len() <= w {
            return None;
        }
        let end = *self.closes.back()?;
        let start = self.closes[self.closes.len() - 1 - w];
        Some((end / start).ln())
    }

    fn range_w(&self, w: usize) -> Option<f64> {
        if self.highs.len() < w {
            return None;
        }
        let slice_start = self.highs.len() - w;
        let max_high = self
            .highs
            .range(slice_start..)
            .copied()
            .fold(f64::MIN, f64::max);
        let min_low = self
            .lows
            .range(slice_start..)
            .copied()
            .fold(f64::MAX, f64::min);
        Some((max_high / min_low).ln())
    }

    fn vol_w(&self, w: usize) -> Option<f64> {
        if self.ret_1s.len() < w {
            return None;
        }
        let start = self.ret_1s.len() - w;
        let window: Vec<f64> = self.ret_1s.range(start..).copied().collect();
        let mean = window.iter().sum::<f64>() / window.len() as f64;
        let variance = window
            .iter()
            .map(|v| {
                let d = *v - mean;
                d * d
            })
            .sum::<f64>()
            / window.len() as f64;
        Some(variance.sqrt())
    }

    fn quote_vol_w(&self, w: usize) -> Option<f64> {
        if self.quote_volumes.len() < w {
            return None;
        }
        let start = self.quote_volumes.len() - w;
        let sum = self.quote_volumes.range(start..).copied().sum::<f64>();
        Some((1.0 + sum).ln())
    }
}

#[derive(Debug, Clone)]
struct TransformState {
    symbol_states: [SymbolRolling; SYMBOL_COUNT],
    max_window: usize,
}

impl TransformState {
    fn new(max_window: usize) -> Self {
        Self {
            symbol_states: [
                SymbolRolling::new(max_window),
                SymbolRolling::new(max_window),
                SymbolRolling::new(max_window),
                SymbolRolling::new(max_window),
            ],
            max_window,
        }
    }

    fn reset_segment(&mut self) {
        for state in &mut self.symbol_states {
            state.reset();
        }
    }
}

pub fn build_feature_schema(cfg: &FeatureTransformConfig) -> FeatureSchema {
    let windows = &cfg.windows_seconds;
    let mut columns = Vec::new();

    for symbol in SYMBOL_CODES {
        columns.push(FeatureColumn {
            name: format!("{symbol}_ret_1s"),
            dtype: FeatureDType::F64,
        });
        for window in windows {
            columns.push(FeatureColumn {
                name: format!("{symbol}_ret_{window}s"),
                dtype: FeatureDType::F64,
            });
        }
        for window in windows {
            columns.push(FeatureColumn {
                name: format!("{symbol}_range_{window}s"),
                dtype: FeatureDType::F64,
            });
        }
        for window in windows {
            columns.push(FeatureColumn {
                name: format!("{symbol}_vol_{window}s"),
                dtype: FeatureDType::F64,
            });
        }
        for window in windows {
            columns.push(FeatureColumn {
                name: format!("{symbol}_quote_vol_{window}s"),
                dtype: FeatureDType::F64,
            });
        }
    }

    columns.push(FeatureColumn {
        name: "tow_sin".to_string(),
        dtype: FeatureDType::F64,
    });
    columns.push(FeatureColumn {
        name: "tow_cos".to_string(),
        dtype: FeatureDType::F64,
    });

    let fingerprint = schema_fingerprint(cfg, &columns);

    info!(
        component = "features",
        event = "features.schema.built",
        version = cfg.schema_version,
        windows = ?cfg.windows_seconds,
        column_count = columns.len(),
        fingerprint = fingerprint
    );

    FeatureSchema {
        version: cfg.schema_version,
        fingerprint,
        columns,
    }
}

pub fn transform_store_range(
    store_path: &Path,
    req: &FeatureTransformRequest,
    cfg: &FeatureTransformConfig,
) -> Result<(FeatureSchema, Vec<FeatureRow>, FeatureTransformReport), FeatureError> {
    validate_request(req)?;
    validate_config(cfg)?;

    info!(
        component = "features",
        event = "features.transform.start",
        store_path = %store_path.display(),
        start_ts_ms_utc = req.start_ts_ms_utc,
        end_ts_ms_utc_exclusive = req.end_ts_ms_utc_exclusive,
        windows = ?cfg.windows_seconds,
        gap_policy = ?cfg.gap_policy
    );

    let schema = build_feature_schema(cfg);
    let conn = Connection::open(store_path)?;
    let mut stmt = conn.prepare(
        "
        SELECT
            open_time_ms,
            symbol_id,
            high,
            low,
            close,
            quote_asset_volume
        FROM klines_1s
        WHERE open_time_ms >= ?1
          AND open_time_ms < ?2
        ORDER BY open_time_ms ASC, symbol_id ASC
        ",
    )?;

    let mut rows = stmt.query(params![req.start_ts_ms_utc, req.end_ts_ms_utc_exclusive])?;
    let mut report = FeatureTransformReport {
        input_points: expected_points(req.start_ts_ms_utc, req.end_ts_ms_utc_exclusive),
        output_points: 0,
        skipped_points: 0,
        gap_ranges: Vec::new(),
        first_error: None,
    };

    let max_window = cfg.windows_seconds.iter().copied().max().unwrap_or(1) as usize;
    let windows_usize: Vec<usize> = cfg
        .windows_seconds
        .iter()
        .copied()
        .map(|w| w as usize)
        .collect();
    let mut state = TransformState::new(max_window.max(1));

    let mut current_frame: Option<Frame> = None;
    let mut last_seen_ts: Option<i64> = None;
    let mut output_rows = Vec::new();

    while let Some(row) = rows.next()? {
        let ts_ms_utc: i64 = row.get(0)?;
        let symbol_id: i64 = row.get(1)?;
        let point = KlinePoint {
            high: row.get(2)?,
            low: row.get(3)?,
            close: row.get(4)?,
            quote_asset_volume: row.get(5)?,
        };
        if ts_ms_utc % STEP_MS != 0 {
            return Err(FeatureError::InvalidTimestamp(ts_ms_utc));
        }

        match current_frame.as_mut() {
            Some(frame) if frame.ts_ms_utc == ts_ms_utc => {
                insert_frame_point(frame, symbol_id, ts_ms_utc, point)?;
            }
            Some(frame) => {
                process_frame(
                    frame,
                    req,
                    cfg,
                    &mut state,
                    &windows_usize,
                    &mut last_seen_ts,
                    &mut report,
                    &mut output_rows,
                )?;

                let mut next_frame = Frame::new(ts_ms_utc);
                insert_frame_point(&mut next_frame, symbol_id, ts_ms_utc, point)?;
                *frame = next_frame;
            }
            None => {
                let mut frame = Frame::new(ts_ms_utc);
                insert_frame_point(&mut frame, symbol_id, ts_ms_utc, point)?;
                current_frame = Some(frame);
            }
        }
    }

    if let Some(frame) = current_frame.take() {
        process_frame(
            &frame,
            req,
            cfg,
            &mut state,
            &windows_usize,
            &mut last_seen_ts,
            &mut report,
            &mut output_rows,
        )?;
    }

    match last_seen_ts {
        Some(last_ts) => {
            let expected_last = req.end_ts_ms_utc_exclusive - STEP_MS;
            if last_ts < expected_last {
                handle_gap(
                    req.end_ts_ms_utc_exclusive,
                    last_ts + STEP_MS,
                    req.end_ts_ms_utc_exclusive,
                    cfg,
                    &mut report,
                    &mut state,
                )?;
            }
        }
        None => {
            if report.input_points > 0 {
                handle_gap(
                    req.start_ts_ms_utc,
                    req.start_ts_ms_utc,
                    req.end_ts_ms_utc_exclusive,
                    cfg,
                    &mut report,
                    &mut state,
                )?;
            }
        }
    }

    report.output_points = output_rows.len() as u64;

    info!(
        component = "features",
        event = "features.transform.finish",
        input_points = report.input_points,
        output_points = report.output_points,
        skipped_points = report.skipped_points,
        gap_ranges_reported = report.gap_ranges.len()
    );

    Ok((schema, output_rows, report))
}

pub fn transform_store_range_for_training(
    store_path: &Path,
    req: &FeatureTransformRequest,
    cfg: &FeatureTransformConfig,
) -> Result<(FeatureSchema, Vec<FeatureRow>, FeatureTransformReport), FeatureError> {
    transform_store_range(store_path, req, cfg)
}

pub fn transform_store_range_for_runtime_cold_start(
    store_path: &Path,
    req: &FeatureTransformRequest,
    cfg: &FeatureTransformConfig,
) -> Result<(FeatureSchema, Vec<FeatureRow>, FeatureTransformReport), FeatureError> {
    transform_store_range(store_path, req, cfg)
}

pub fn horizon_conditioning(
    horizon_seconds: u32,
    max_duration_seconds: u32,
) -> HorizonConditioning {
    if max_duration_seconds == 0 {
        return HorizonConditioning {
            log_horizon_norm: 0.0,
            sqrt_horizon_norm: 0.0,
        };
    }

    let horizon = horizon_seconds as f64;
    let max_duration = max_duration_seconds as f64;
    let log_horizon = (1.0 + horizon).ln();
    let log_max = (1.0 + max_duration).ln();
    let log_horizon_norm = if log_max > 0.0 {
        log_horizon / log_max
    } else {
        0.0
    };
    let sqrt_horizon_norm = horizon.sqrt() / max_duration.sqrt();

    HorizonConditioning {
        log_horizon_norm,
        sqrt_horizon_norm,
    }
}

pub fn assert_schema_compatible(
    expected_version: u32,
    expected_fingerprint: &str,
    actual: &FeatureSchema,
) -> Result<(), FeatureError> {
    if expected_version != actual.version {
        return Err(FeatureError::SchemaVersionMismatch {
            expected: expected_version,
            actual: actual.version,
        });
    }

    if expected_fingerprint != actual.fingerprint {
        return Err(FeatureError::SchemaFingerprintMismatch {
            expected: expected_fingerprint.to_string(),
            actual: actual.fingerprint.clone(),
        });
    }

    Ok(())
}

fn validate_request(req: &FeatureTransformRequest) -> Result<(), FeatureError> {
    if req.end_ts_ms_utc_exclusive <= req.start_ts_ms_utc {
        return Err(FeatureError::InvalidRequest(
            "end timestamp must be greater than start timestamp".to_string(),
        ));
    }

    if req.start_ts_ms_utc % STEP_MS != 0 || req.end_ts_ms_utc_exclusive % STEP_MS != 0 {
        return Err(FeatureError::InvalidRequest(
            "request boundaries must be aligned to 1000ms".to_string(),
        ));
    }

    Ok(())
}

fn validate_config(cfg: &FeatureTransformConfig) -> Result<(), FeatureError> {
    if cfg.max_duration_seconds == 0 {
        return Err(FeatureError::InvalidConfig(
            "max_duration_seconds must be > 0".to_string(),
        ));
    }

    if cfg.schema_version != FEATURE_SCHEMA_VERSION {
        return Err(FeatureError::InvalidConfig(format!(
            "schema_version must equal FEATURE_SCHEMA_VERSION ({FEATURE_SCHEMA_VERSION})"
        )));
    }

    let mut seen = HashSet::new();
    for window in &cfg.windows_seconds {
        if *window == 0 {
            return Err(FeatureError::InvalidConfig(
                "windows_seconds entries must be > 0".to_string(),
            ));
        }
        if !seen.insert(*window) {
            return Err(FeatureError::InvalidConfig(
                "windows_seconds entries must be unique".to_string(),
            ));
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_frame(
    frame: &Frame,
    req: &FeatureTransformRequest,
    cfg: &FeatureTransformConfig,
    state: &mut TransformState,
    windows: &[usize],
    last_seen_ts: &mut Option<i64>,
    report: &mut FeatureTransformReport,
    output_rows: &mut Vec<FeatureRow>,
) -> Result<(), FeatureError> {
    if frame.ts_ms_utc < req.start_ts_ms_utc || frame.ts_ms_utc >= req.end_ts_ms_utc_exclusive {
        return Ok(());
    }

    if let Some(prev_seen) = *last_seen_ts {
        let expected = prev_seen + STEP_MS;
        if frame.ts_ms_utc != expected {
            handle_gap(expected, expected, frame.ts_ms_utc, cfg, report, state)?;
        }
    } else if frame.ts_ms_utc > req.start_ts_ms_utc {
        handle_gap(
            req.start_ts_ms_utc,
            req.start_ts_ms_utc,
            frame.ts_ms_utc,
            cfg,
            report,
            state,
        )?;
    }

    *last_seen_ts = Some(frame.ts_ms_utc);

    if let Some(missing_symbols) = missing_symbols(frame) {
        let reason = format!(
            "incomplete frame at {} missing={missing_symbols:?}",
            frame.ts_ms_utc
        );
        handle_incomplete_frame(frame.ts_ms_utc, missing_symbols, cfg, report, state, reason)?;
        return Ok(());
    }

    for (idx, slot) in frame.points.iter().enumerate() {
        let point = slot.expect("frame completeness checked");
        state.symbol_states[idx].push(point);
    }

    if !is_warm(state, windows) {
        return Ok(());
    }

    let mut values = Vec::new();
    for symbol_state in &state.symbol_states {
        values.push(symbol_state.ret_1s().expect("warm state must have ret_1s"));
        for w in windows {
            values.push(symbol_state.ret_w(*w).expect("warm state must have ret_w"));
        }
        for w in windows {
            values.push(
                symbol_state
                    .range_w(*w)
                    .expect("warm state must have range_w"),
            );
        }
        for w in windows {
            values.push(symbol_state.vol_w(*w).expect("warm state must have vol_w"));
        }
        for w in windows {
            values.push(
                symbol_state
                    .quote_vol_w(*w)
                    .expect("warm state must have quote_vol_w"),
            );
        }
    }

    let (tow_sin, tow_cos) = time_of_week_encoding(frame.ts_ms_utc)?;
    values.push(tow_sin);
    values.push(tow_cos);

    output_rows.push(FeatureRow {
        ts_ms_utc: frame.ts_ms_utc,
        values,
    });

    Ok(())
}

fn insert_frame_point(
    frame: &mut Frame,
    symbol_id: i64,
    ts_ms_utc: i64,
    point: KlinePoint,
) -> Result<(), FeatureError> {
    let index = symbol_index(symbol_id).ok_or(FeatureError::UnknownSymbolId {
        symbol_id,
        ts_ms_utc,
    })?;
    if frame.points[index].is_some() {
        return Err(FeatureError::DuplicateSymbolInFrame {
            symbol_id,
            ts_ms_utc,
        });
    }
    frame.points[index] = Some(point);
    Ok(())
}

fn symbol_index(symbol_id: i64) -> Option<usize> {
    match symbol_id {
        1 => Some(0),
        2 => Some(1),
        3 => Some(2),
        4 => Some(3),
        _ => None,
    }
}

fn missing_symbols(frame: &Frame) -> Option<Vec<String>> {
    let mut missing = Vec::new();
    for (idx, point) in frame.points.iter().enumerate() {
        if point.is_none() {
            missing.push(SYMBOL_CODES[idx].to_uppercase());
        }
    }
    if missing.is_empty() {
        None
    } else {
        Some(missing)
    }
}

fn is_warm(state: &TransformState, windows: &[usize]) -> bool {
    let required = state.max_window.max(1);
    for symbol_state in &state.symbol_states {
        if symbol_state.closes.len() <= required {
            return false;
        }
        if symbol_state.ret_1s.len() < required {
            return false;
        }
        for w in windows {
            if symbol_state.closes.len() <= *w
                || symbol_state.highs.len() < *w
                || symbol_state.quote_volumes.len() < *w
                || symbol_state.ret_1s.len() < *w
            {
                return false;
            }
        }
    }
    true
}

fn time_of_week_encoding(ts_ms_utc: i64) -> Result<(f64, f64), FeatureError> {
    let dt = Utc
        .timestamp_millis_opt(ts_ms_utc)
        .single()
        .ok_or(FeatureError::InvalidTimestamp(ts_ms_utc))?;
    let weekday = dt.weekday().num_days_from_monday() as f64;
    let seconds_of_day = dt.hour() as f64 * 3600.0 + dt.minute() as f64 * 60.0 + dt.second() as f64;
    let seconds_of_week = weekday * 86_400.0 + seconds_of_day;
    let angle = 2.0 * PI * (seconds_of_week / WEEK_SECONDS);
    Ok((angle.sin(), angle.cos()))
}

fn handle_incomplete_frame(
    ts_ms_utc: i64,
    missing_symbols: Vec<String>,
    cfg: &FeatureTransformConfig,
    report: &mut FeatureTransformReport,
    state: &mut TransformState,
    reason: String,
) -> Result<(), FeatureError> {
    match cfg.gap_policy {
        GapPolicy::Strict => Err(FeatureError::IncompleteFrame {
            ts_ms_utc,
            missing_symbols,
        }),
        GapPolicy::ReportAndSkip => {
            warn!(
                component = "features",
                event = "features.transform.gap_detected",
                ts_ms_utc = ts_ms_utc,
                reason = "incomplete_frame",
                details = reason
            );
            update_report_with_gap(report, ts_ms_utc, ts_ms_utc + STEP_MS);
            state.reset_segment();
            if report.first_error.is_none() {
                report.first_error = Some(format!("incomplete frame at {ts_ms_utc}"));
            }
            Ok(())
        }
    }
}

fn handle_gap(
    expected_next_ts_ms_utc: i64,
    start_ts_ms_utc: i64,
    end_ts_ms_utc_exclusive: i64,
    cfg: &FeatureTransformConfig,
    report: &mut FeatureTransformReport,
    state: &mut TransformState,
) -> Result<(), FeatureError> {
    if end_ts_ms_utc_exclusive <= start_ts_ms_utc {
        return Ok(());
    }

    let missing_points = expected_points(start_ts_ms_utc, end_ts_ms_utc_exclusive);
    match cfg.gap_policy {
        GapPolicy::Strict => Err(FeatureError::ContinuityGap {
            expected_next_ts_ms_utc,
            actual_ts_ms_utc: end_ts_ms_utc_exclusive,
            missing_points,
        }),
        GapPolicy::ReportAndSkip => {
            warn!(
                component = "features",
                event = "features.transform.gap_detected",
                expected_next_ts_ms_utc = expected_next_ts_ms_utc,
                start_ts_ms_utc = start_ts_ms_utc,
                end_ts_ms_utc_exclusive = end_ts_ms_utc_exclusive,
                missing_points = missing_points
            );
            update_report_with_gap(report, start_ts_ms_utc, end_ts_ms_utc_exclusive);
            state.reset_segment();
            if report.first_error.is_none() {
                report.first_error = Some(format!(
                    "continuity gap from {start_ts_ms_utc} to {end_ts_ms_utc_exclusive}"
                ));
            }
            Ok(())
        }
    }
}

fn update_report_with_gap(
    report: &mut FeatureTransformReport,
    start_ts_ms_utc: i64,
    end_ts_ms_utc_exclusive: i64,
) {
    if end_ts_ms_utc_exclusive <= start_ts_ms_utc {
        return;
    }
    report.skipped_points = report
        .skipped_points
        .saturating_add(expected_points(start_ts_ms_utc, end_ts_ms_utc_exclusive));
    if report.gap_ranges.len() < MAX_REPORTED_GAP_RANGES {
        report
            .gap_ranges
            .push((start_ts_ms_utc, end_ts_ms_utc_exclusive));
    }
}

fn expected_points(start_ts_ms_utc: i64, end_ts_ms_utc_exclusive: i64) -> u64 {
    if end_ts_ms_utc_exclusive <= start_ts_ms_utc {
        0
    } else {
        ((end_ts_ms_utc_exclusive - start_ts_ms_utc) / STEP_MS) as u64
    }
}

fn schema_fingerprint(cfg: &FeatureTransformConfig, columns: &[FeatureColumn]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("version:{};", cfg.schema_version));
    hasher.update(format!(
        "max_duration_seconds:{};",
        cfg.max_duration_seconds
    ));
    hasher.update("windows:");
    for window in &cfg.windows_seconds {
        hasher.update(format!("{window},"));
    }
    hasher.update(";columns:");
    for column in columns {
        hasher.update(column.name.as_bytes());
        hasher.update(":f64;");
    }
    hex::encode(hasher.finalize())
}

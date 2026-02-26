//! Step 8 historical Binance 1s kline loading.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use csv::StringRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{debug, info, warn};
use zip::ZipArchive;

const BINANCE_DATA_BASE_URL: &str = "https://data.binance.vision/data/spot";
const STEP_MS: i64 = 1_000;
const MAX_REPORTED_GAP_RANGES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinanceSymbol {
    BtcUsdt,
    EthUsdt,
    SolUsdt,
    XrpUsdt,
}

impl BinanceSymbol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BtcUsdt => "BTCUSDT",
            Self::EthUsdt => "ETHUSDT",
            Self::SolUsdt => "SOLUSDT",
            Self::XrpUsdt => "XRPUSDT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArchiveKind {
    Monthly,
    Daily,
}

impl ArchiveKind {
    fn as_path_segment(self) -> &'static str {
        match self {
            Self::Monthly => "monthly",
            Self::Daily => "daily",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveRef {
    pub kind: ArchiveKind,
    pub symbol: BinanceSymbol,
    pub period_start_ts_ms_utc: i64,
    pub period_end_ts_ms_utc_exclusive: i64,
    pub url: String,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalArchiveSource {
    Cached,
    Downloaded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalArchive {
    pub archive: ArchiveRef,
    pub local_path: PathBuf,
    pub source: LocalArchiveSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kline1s {
    pub open_time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub close_time_ms: i64,
    pub quote_asset_volume: f64,
    pub trade_count: u64,
    pub taker_buy_base_volume: f64,
    pub taker_buy_quote_volume: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KlineLoadRequest {
    pub symbol: BinanceSymbol,
    pub start_ts_ms_utc: i64,
    pub end_ts_ms_utc_exclusive: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KlineCoverageReport {
    pub expected_points: u64,
    pub actual_points: u64,
    pub missing_points: u64,
    pub duplicate_points_removed: u64,
    pub total_gap_ranges: u64,
    pub gap_ranges: Vec<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KlineLoadResult {
    pub symbol: BinanceSymbol,
    pub rows: Vec<Kline1s>,
    pub coverage: KlineCoverageReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalKlinesConfig {
    pub data_root: PathBuf,
    pub http_timeout_ms: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub verify_checksum: bool,
}

impl Default for HistoricalKlinesConfig {
    fn default() -> Self {
        Self {
            data_root: PathBuf::from("data/binance"),
            http_timeout_ms: 15_000,
            max_retries: 2,
            retry_backoff_ms: 200,
            verify_checksum: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum KlineLoadError {
    #[error("invalid kline load request: {0}")]
    InvalidRequest(String),
    #[error("invalid timestamp in request: {0}")]
    InvalidTimestamp(i64),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP client build error: {0}")]
    HttpClientBuild(String),
    #[error("HTTP request failed for {url}: {message}")]
    HttpRequest { url: String, message: String },
    #[error("archive at {path} has no entries")]
    EmptyZipArchive { path: PathBuf },
    #[error("archive at {path} has no CSV entry")]
    MissingCsvEntry { path: PathBuf },
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("kline record has {found} columns, expected at least {expected}")]
    InvalidRecordColumns { found: usize, expected: usize },
    #[error("failed to parse field {field} value '{value}'")]
    ParseField { field: &'static str, value: String },
    #[error("invalid checksum payload for {url}: {payload}")]
    InvalidChecksumPayload { url: String, payload: String },
    #[error("checksum mismatch for {path}: expected {expected}, actual {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
}

pub fn plan_required_archives(req: &KlineLoadRequest) -> Vec<ArchiveRef> {
    let Some((start_dt, end_dt)) = request_bounds(req) else {
        return Vec::new();
    };

    let last_inclusive = match end_dt.checked_sub_signed(ChronoDuration::milliseconds(1)) {
        Some(ts) => ts,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut month = NaiveDate::from_ymd_opt(start_dt.year(), start_dt.month(), 1)
        .expect("valid month start date expected");
    let end_month = NaiveDate::from_ymd_opt(last_inclusive.year(), last_inclusive.month(), 1)
        .expect("valid month start date expected");

    while month <= end_month {
        let month_start = day_start_ms(month);
        let month_end = day_start_ms(next_month(month));
        let month_full_in_range =
            req.start_ts_ms_utc <= month_start && req.end_ts_ms_utc_exclusive >= month_end;

        if month_full_in_range {
            out.push(monthly_archive(req.symbol, month, month_start, month_end));
        } else {
            let mut day = month;
            let next_month_date = next_month(month);
            while day < next_month_date {
                let day_start = day_start_ms(day);
                let day_end = day_start_ms(day.succ_opt().expect("next day should exist"));
                let intersects =
                    req.start_ts_ms_utc < day_end && day_start < req.end_ts_ms_utc_exclusive;
                if intersects {
                    out.push(daily_archive(req.symbol, day, day_start, day_end));
                }
                day = day.succ_opt().expect("next day should exist");
            }
        }

        month = next_month(month);
    }

    out
}

pub fn sync_archives(
    req: &KlineLoadRequest,
    cfg: &HistoricalKlinesConfig,
) -> Result<Vec<LocalArchive>, KlineLoadError> {
    validate_request(req)?;
    let archives = plan_required_archives(req);
    info!(
        component = "binance_klines",
        event = "binance.sync.start",
        symbol = req.symbol.as_str(),
        archive_count = archives.len(),
        verify_checksum = cfg.verify_checksum
    );

    let fetcher = ReqwestBlockingFetcher::new(cfg.http_timeout_ms)?;
    sync_archives_with_fetcher(&archives, cfg, &fetcher)
}

pub fn load_1s_klines(
    req: &KlineLoadRequest,
    cfg: &HistoricalKlinesConfig,
) -> Result<KlineLoadResult, KlineLoadError> {
    validate_request(req)?;
    let local_archives = sync_archives(req, cfg)?;

    let mut all_rows = Vec::new();
    for archive in &local_archives {
        let mut parsed = parse_zip_archive(&archive.local_path, req)?;
        all_rows.append(&mut parsed);
    }

    all_rows.sort_by_key(|row| row.open_time_ms);

    let mut deduped = Vec::with_capacity(all_rows.len());
    let mut duplicates_removed = 0u64;
    for row in all_rows {
        if deduped
            .last()
            .map(|existing: &Kline1s| existing.open_time_ms == row.open_time_ms)
            .unwrap_or(false)
        {
            duplicates_removed += 1;
        } else {
            deduped.push(row);
        }
    }

    let coverage = compute_coverage(req, &deduped, duplicates_removed);
    if coverage.missing_points > 0 {
        info!(
            component = "binance_klines",
            event = "binance.load.gap_detected",
            symbol = req.symbol.as_str(),
            missing_points = coverage.missing_points,
            total_gap_ranges = coverage.total_gap_ranges,
            reported_gap_ranges = coverage.gap_ranges.len()
        );
    }

    info!(
        component = "binance_klines",
        event = "binance.load.finish",
        symbol = req.symbol.as_str(),
        expected_points = coverage.expected_points,
        actual_points = coverage.actual_points,
        missing_points = coverage.missing_points,
        duplicate_points_removed = coverage.duplicate_points_removed
    );

    Ok(KlineLoadResult {
        symbol: req.symbol,
        rows: deduped,
        coverage,
    })
}

fn sync_archives_with_fetcher(
    archives: &[ArchiveRef],
    cfg: &HistoricalKlinesConfig,
    fetcher: &dyn HttpFetcher,
) -> Result<Vec<LocalArchive>, KlineLoadError> {
    let mut local = Vec::with_capacity(archives.len());

    for archive in archives {
        let local_path = cfg.data_root.join(&archive.relative_path);
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut expected_checksum = None;
        if local_path.exists() {
            if cfg.verify_checksum {
                let checksum_url = format!("{}.CHECKSUM", archive.url);
                let expected = fetch_checksum_with_retry(fetcher, &checksum_url, cfg)?;
                let actual = file_sha256_hex(&local_path)?;
                if actual.eq_ignore_ascii_case(&expected) {
                    info!(
                        component = "binance_klines",
                        event = "binance.sync.file.cached",
                        symbol = archive.symbol.as_str(),
                        kind = archive.kind.as_path_segment(),
                        path = %local_path.display()
                    );
                    local.push(LocalArchive {
                        archive: archive.clone(),
                        local_path,
                        source: LocalArchiveSource::Cached,
                    });
                    continue;
                }

                warn!(
                    component = "binance_klines",
                    event = "binance.sync.file.checksum_failed",
                    symbol = archive.symbol.as_str(),
                    kind = archive.kind.as_path_segment(),
                    path = %local_path.display(),
                    expected = %expected,
                    actual = %actual
                );
                expected_checksum = Some(expected);
            } else {
                info!(
                    component = "binance_klines",
                    event = "binance.sync.file.cached",
                    symbol = archive.symbol.as_str(),
                    kind = archive.kind.as_path_segment(),
                    path = %local_path.display()
                );
                local.push(LocalArchive {
                    archive: archive.clone(),
                    local_path,
                    source: LocalArchiveSource::Cached,
                });
                continue;
            }
        }

        let bytes = fetch_bytes_with_retry(fetcher, &archive.url, cfg)?;
        write_atomic(&local_path, &bytes)?;

        if cfg.verify_checksum {
            let expected = match expected_checksum {
                Some(value) => value,
                None => {
                    let checksum_url = format!("{}.CHECKSUM", archive.url);
                    fetch_checksum_with_retry(fetcher, &checksum_url, cfg)?
                }
            };
            let actual = file_sha256_hex(&local_path)?;
            if !actual.eq_ignore_ascii_case(&expected) {
                warn!(
                    component = "binance_klines",
                    event = "binance.sync.file.checksum_failed",
                    symbol = archive.symbol.as_str(),
                    kind = archive.kind.as_path_segment(),
                    path = %local_path.display(),
                    expected = %expected,
                    actual = %actual
                );
                return Err(KlineLoadError::ChecksumMismatch {
                    path: local_path,
                    expected,
                    actual,
                });
            }
        }

        info!(
            component = "binance_klines",
            event = "binance.sync.file.downloaded",
            symbol = archive.symbol.as_str(),
            kind = archive.kind.as_path_segment(),
            path = %local_path.display(),
            bytes = bytes.len()
        );
        debug!(
            component = "binance_klines",
            event = "binance.sync.file.downloaded.debug",
            url = %archive.url
        );

        local.push(LocalArchive {
            archive: archive.clone(),
            local_path,
            source: LocalArchiveSource::Downloaded,
        });
    }

    Ok(local)
}

fn parse_zip_archive(path: &Path, req: &KlineLoadRequest) -> Result<Vec<Kline1s>, KlineLoadError> {
    let file = fs::File::open(path)?;
    let mut zip = ZipArchive::new(file)?;
    if zip.is_empty() {
        return Err(KlineLoadError::EmptyZipArchive {
            path: path.to_path_buf(),
        });
    }

    let mut csv_buf = None;
    for idx in 0..zip.len() {
        let mut entry = zip.by_index(idx)?;
        if entry.is_dir() {
            continue;
        }
        if !entry.name().to_ascii_lowercase().ends_with(".csv") {
            continue;
        }

        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        csv_buf = Some(buf);
        break;
    }
    let csv_buf = csv_buf.ok_or_else(|| KlineLoadError::MissingCsvEntry {
        path: path.to_path_buf(),
    })?;

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(Cursor::new(csv_buf));

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        let row = parse_kline_record(&record)?;
        if row.open_time_ms >= req.start_ts_ms_utc && row.open_time_ms < req.end_ts_ms_utc_exclusive
        {
            rows.push(row);
        }
    }

    Ok(rows)
}

fn parse_kline_record(record: &StringRecord) -> Result<Kline1s, KlineLoadError> {
    if record.len() < 11 {
        return Err(KlineLoadError::InvalidRecordColumns {
            found: record.len(),
            expected: 11,
        });
    }

    let open_time_raw = parse_i64(record, 0, "open_time_ms")?;
    let close_time_raw = parse_i64(record, 6, "close_time_ms")?;

    Ok(Kline1s {
        open_time_ms: normalize_to_millis(open_time_raw),
        open: parse_f64(record, 1, "open")?,
        high: parse_f64(record, 2, "high")?,
        low: parse_f64(record, 3, "low")?,
        close: parse_f64(record, 4, "close")?,
        volume: parse_f64(record, 5, "volume")?,
        close_time_ms: normalize_to_millis(close_time_raw),
        quote_asset_volume: parse_f64(record, 7, "quote_asset_volume")?,
        trade_count: parse_u64(record, 8, "trade_count")?,
        taker_buy_base_volume: parse_f64(record, 9, "taker_buy_base_volume")?,
        taker_buy_quote_volume: parse_f64(record, 10, "taker_buy_quote_volume")?,
    })
}

fn normalize_to_millis(ts: i64) -> i64 {
    // Binance 1s historical archives can be emitted in microseconds.
    if ts.abs() >= 1_000_000_000_000_000_000 {
        ts / 1_000_000
    } else if ts.abs() >= 1_000_000_000_000_000 {
        ts / 1_000
    } else {
        ts
    }
}

fn parse_i64(
    record: &StringRecord,
    idx: usize,
    field: &'static str,
) -> Result<i64, KlineLoadError> {
    let raw = record.get(idx).unwrap_or_default();
    raw.parse::<i64>().map_err(|_| KlineLoadError::ParseField {
        field,
        value: raw.to_string(),
    })
}

fn parse_u64(
    record: &StringRecord,
    idx: usize,
    field: &'static str,
) -> Result<u64, KlineLoadError> {
    let raw = record.get(idx).unwrap_or_default();
    raw.parse::<u64>().map_err(|_| KlineLoadError::ParseField {
        field,
        value: raw.to_string(),
    })
}

fn parse_f64(
    record: &StringRecord,
    idx: usize,
    field: &'static str,
) -> Result<f64, KlineLoadError> {
    let raw = record.get(idx).unwrap_or_default();
    raw.parse::<f64>().map_err(|_| KlineLoadError::ParseField {
        field,
        value: raw.to_string(),
    })
}

fn compute_coverage(
    req: &KlineLoadRequest,
    rows: &[Kline1s],
    duplicate_points_removed: u64,
) -> KlineCoverageReport {
    let expected_points = ((req.end_ts_ms_utc_exclusive - req.start_ts_ms_utc) / STEP_MS) as u64;
    let actual_points = rows.len() as u64;
    let (gap_ranges, total_gap_ranges, missing_points) = gap_ranges(req, rows);

    KlineCoverageReport {
        expected_points,
        actual_points,
        missing_points,
        duplicate_points_removed,
        total_gap_ranges: total_gap_ranges as u64,
        gap_ranges,
    }
}

fn gap_ranges(req: &KlineLoadRequest, rows: &[Kline1s]) -> (Vec<(i64, i64)>, usize, u64) {
    if req.end_ts_ms_utc_exclusive <= req.start_ts_ms_utc {
        return (Vec::new(), 0, 0);
    }

    let mut full = Vec::new();
    let mut cursor = req.start_ts_ms_utc;

    for row in rows {
        if row.open_time_ms > cursor {
            full.push((cursor, row.open_time_ms - STEP_MS));
        }
        cursor = row.open_time_ms.saturating_add(STEP_MS);
    }

    if cursor < req.end_ts_ms_utc_exclusive {
        full.push((cursor, req.end_ts_ms_utc_exclusive - STEP_MS));
    }

    let missing_points = full
        .iter()
        .map(|(start, end)| ((end - start) / STEP_MS + 1) as u64)
        .sum();

    let total = full.len();
    let reported = full.into_iter().take(MAX_REPORTED_GAP_RANGES).collect();

    (reported, total, missing_points)
}

fn validate_request(req: &KlineLoadRequest) -> Result<(), KlineLoadError> {
    if req.end_ts_ms_utc_exclusive <= req.start_ts_ms_utc {
        return Err(KlineLoadError::InvalidRequest(
            "end_ts_ms_utc_exclusive must be greater than start_ts_ms_utc".to_string(),
        ));
    }
    if req.start_ts_ms_utc % STEP_MS != 0 || req.end_ts_ms_utc_exclusive % STEP_MS != 0 {
        return Err(KlineLoadError::InvalidRequest(
            "start/end timestamps must be 1000ms aligned".to_string(),
        ));
    }
    request_bounds(req).ok_or(KlineLoadError::InvalidTimestamp(req.start_ts_ms_utc))?;
    Ok(())
}

fn request_bounds(
    req: &KlineLoadRequest,
) -> Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> {
    let start = Utc.timestamp_millis_opt(req.start_ts_ms_utc).single()?;
    let end = Utc
        .timestamp_millis_opt(req.end_ts_ms_utc_exclusive)
        .single()?;
    Some((start, end))
}

fn monthly_archive(
    symbol: BinanceSymbol,
    month_start_date: NaiveDate,
    start_ms: i64,
    end_ms: i64,
) -> ArchiveRef {
    let symbol_text = symbol.as_str();
    let filename = format!(
        "{symbol_text}-1s-{:04}-{:02}.zip",
        month_start_date.year(),
        month_start_date.month()
    );
    let url = format!("{BINANCE_DATA_BASE_URL}/monthly/klines/{symbol_text}/1s/{filename}");

    ArchiveRef {
        kind: ArchiveKind::Monthly,
        symbol,
        period_start_ts_ms_utc: start_ms,
        period_end_ts_ms_utc_exclusive: end_ms,
        url,
        relative_path: PathBuf::from(format!("{symbol_text}/1s/monthly/{filename}")),
    }
}

fn daily_archive(symbol: BinanceSymbol, date: NaiveDate, start_ms: i64, end_ms: i64) -> ArchiveRef {
    let symbol_text = symbol.as_str();
    let filename = format!(
        "{symbol_text}-1s-{:04}-{:02}-{:02}.zip",
        date.year(),
        date.month(),
        date.day()
    );
    let url = format!("{BINANCE_DATA_BASE_URL}/daily/klines/{symbol_text}/1s/{filename}");

    ArchiveRef {
        kind: ArchiveKind::Daily,
        symbol,
        period_start_ts_ms_utc: start_ms,
        period_end_ts_ms_utc_exclusive: end_ms,
        url,
        relative_path: PathBuf::from(format!("{symbol_text}/1s/daily/{filename}")),
    }
}

fn day_start_ms(date: NaiveDate) -> i64 {
    Utc.with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .expect("valid UTC day boundary expected")
        .timestamp_millis()
}

fn next_month(date: NaiveDate) -> NaiveDate {
    if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1).expect("valid next month expected")
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
            .expect("valid next month expected")
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), KlineLoadError> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| {
            KlineLoadError::InvalidRequest(format!("invalid output path: {}", path.display()))
        })?;
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));

    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    fs::rename(tmp_path, path)?;
    Ok(())
}

fn file_sha256_hex(path: &Path) -> Result<String, KlineLoadError> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn parse_checksum_payload(url: &str, payload: &[u8]) -> Result<String, KlineLoadError> {
    let text = String::from_utf8_lossy(payload);
    let token =
        text.split_whitespace()
            .next()
            .ok_or_else(|| KlineLoadError::InvalidChecksumPayload {
                url: url.to_string(),
                payload: text.trim().to_string(),
            })?;

    if token.len() != 64 || hex::decode(token).is_err() {
        return Err(KlineLoadError::InvalidChecksumPayload {
            url: url.to_string(),
            payload: text.trim().to_string(),
        });
    }

    Ok(token.to_ascii_lowercase())
}

trait HttpFetcher {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, KlineLoadError>;
}

struct ReqwestBlockingFetcher {
    client: reqwest::blocking::Client,
}

impl ReqwestBlockingFetcher {
    fn new(timeout_ms: u64) -> Result<Self, KlineLoadError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| KlineLoadError::HttpClientBuild(err.to_string()))?;
        Ok(Self { client })
    }
}

impl HttpFetcher for ReqwestBlockingFetcher {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, KlineLoadError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|err| KlineLoadError::HttpRequest {
                url: url.to_string(),
                message: err.to_string(),
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(KlineLoadError::HttpRequest {
                url: url.to_string(),
                message: format!("unexpected HTTP status {status}"),
            });
        }

        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|err| KlineLoadError::HttpRequest {
                url: url.to_string(),
                message: err.to_string(),
            })
    }
}

fn fetch_bytes_with_retry(
    fetcher: &dyn HttpFetcher,
    url: &str,
    cfg: &HistoricalKlinesConfig,
) -> Result<Vec<u8>, KlineLoadError> {
    retry(cfg, || fetcher.get_bytes(url))
}

fn fetch_checksum_with_retry(
    fetcher: &dyn HttpFetcher,
    checksum_url: &str,
    cfg: &HistoricalKlinesConfig,
) -> Result<String, KlineLoadError> {
    let payload = fetch_bytes_with_retry(fetcher, checksum_url, cfg)?;
    parse_checksum_payload(checksum_url, &payload)
}

fn retry<T>(
    cfg: &HistoricalKlinesConfig,
    mut f: impl FnMut() -> Result<T, KlineLoadError>,
) -> Result<T, KlineLoadError> {
    let mut attempt: u32 = 0;
    loop {
        match f() {
            Ok(value) => return Ok(value),
            Err(err) if attempt >= cfg.max_retries => return Err(err),
            Err(_) => {
                attempt = attempt.saturating_add(1);
                let shift = attempt.saturating_sub(1).min(10);
                let factor = 1u64 << shift;
                let sleep_ms = cfg.retry_backoff_ms.saturating_mul(factor);
                std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    #[derive(Default)]
    struct MockFetcher {
        responses: HashMap<String, Vec<u8>>,
    }

    impl MockFetcher {
        fn with(mut self, url: &str, body: &[u8]) -> Self {
            self.responses.insert(url.to_string(), body.to_vec());
            self
        }
    }

    impl HttpFetcher for MockFetcher {
        fn get_bytes(&self, url: &str) -> Result<Vec<u8>, KlineLoadError> {
            self.responses
                .get(url)
                .cloned()
                .ok_or_else(|| KlineLoadError::HttpRequest {
                    url: url.to_string(),
                    message: "missing mock response".to_string(),
                })
        }
    }

    fn sample_req() -> KlineLoadRequest {
        KlineLoadRequest {
            symbol: BinanceSymbol::BtcUsdt,
            start_ts_ms_utc: 1_704_067_200_000,
            end_ts_ms_utc_exclusive: 1_704_067_203_000,
        }
    }

    fn sample_csv() -> &'static str {
        "1704067200000,100,101,99,100.5,10,1704067200999,1005,42,5,502.5,0\n1704067201000,100.5,101.5,100,101,11,1704067201999,1111,43,6,606,0\n1704067202000,101,102,100.5,101.2,12,1704067202999,1214.4,44,7,707,0\n"
    }

    fn write_zip(path: &Path, csv_body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("data.csv", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(csv_body.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn plan_across_month_boundary_is_deterministic() {
        let req = KlineLoadRequest {
            symbol: BinanceSymbol::EthUsdt,
            start_ts_ms_utc: 1_706_745_600_000, // 2024-02-01
            end_ts_ms_utc_exclusive: 1_709_251_200_000, // 2024-03-01
        };

        let archives = plan_required_archives(&req);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].kind, ArchiveKind::Monthly);
        assert!(archives[0]
            .url
            .ends_with("/monthly/klines/ETHUSDT/1s/ETHUSDT-1s-2024-02.zip"));
    }

    #[test]
    fn parse_kline_record_enforces_schema_and_numbers() {
        let record = StringRecord::from(vec![
            "1704067200000",
            "100",
            "101",
            "99",
            "100.5",
            "10",
            "1704067200999",
            "1005",
            "42",
            "5",
            "502.5",
            "0",
        ]);

        let parsed = parse_kline_record(&record).unwrap();
        assert_eq!(parsed.open_time_ms, 1_704_067_200_000);
        assert_eq!(parsed.trade_count, 42);

        let bad = StringRecord::from(vec!["oops"]);
        assert!(matches!(
            parse_kline_record(&bad).unwrap_err(),
            KlineLoadError::InvalidRecordColumns { .. }
        ));
    }

    #[test]
    fn parse_kline_record_normalizes_microsecond_timestamps_to_millis() {
        let record = StringRecord::from(vec![
            "1735689600000000",
            "100",
            "101",
            "99",
            "100.5",
            "10",
            "1735689600999999",
            "1005",
            "42",
            "5",
            "502.5",
            "0",
        ]);

        let parsed = parse_kline_record(&record).unwrap();
        assert_eq!(parsed.open_time_ms, 1_735_689_600_000);
        assert_eq!(parsed.close_time_ms, 1_735_689_600_999);
    }

    #[test]
    fn gap_detection_and_duplicates_are_reported() {
        let req = sample_req();
        let rows = vec![
            Kline1s {
                open_time_ms: 1_704_067_200_000,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
                close_time_ms: 1_704_067_200_999,
                quote_asset_volume: 1.0,
                trade_count: 1,
                taker_buy_base_volume: 1.0,
                taker_buy_quote_volume: 1.0,
            },
            Kline1s {
                open_time_ms: 1_704_067_202_000,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
                close_time_ms: 1_704_067_202_999,
                quote_asset_volume: 1.0,
                trade_count: 1,
                taker_buy_base_volume: 1.0,
                taker_buy_quote_volume: 1.0,
            },
        ];

        let coverage = compute_coverage(&req, &rows, 2);
        assert_eq!(coverage.expected_points, 3);
        assert_eq!(coverage.actual_points, 2);
        assert_eq!(coverage.missing_points, 1);
        assert_eq!(coverage.duplicate_points_removed, 2);
        assert_eq!(
            coverage.gap_ranges,
            vec![(1_704_067_201_000, 1_704_067_201_000)]
        );
    }

    #[test]
    fn checksum_mismatch_is_rejected_when_enabled() {
        let req = sample_req();
        let temp = tempdir().unwrap();
        let cfg = HistoricalKlinesConfig {
            data_root: temp.path().to_path_buf(),
            verify_checksum: true,
            ..HistoricalKlinesConfig::default()
        };
        let archives = plan_required_archives(&req);

        let archive = &archives[0];
        let zip_path = cfg.data_root.join(&archive.relative_path);
        write_zip(&zip_path, sample_csv());

        let checksum_url = format!("{}.CHECKSUM", archive.url);
        let zip_bytes = fs::read(&zip_path).unwrap();
        let fetcher = MockFetcher::default()
            .with(
                &checksum_url,
                b"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  anything.zip\n",
            )
            .with(&archive.url, &zip_bytes);

        let err = sync_archives_with_fetcher(&archives, &cfg, &fetcher).unwrap_err();
        assert!(matches!(err, KlineLoadError::ChecksumMismatch { .. }));
    }

    #[test]
    fn cache_hit_skips_download_when_checksum_verification_disabled() {
        let req = sample_req();
        let temp = tempdir().unwrap();
        let cfg = HistoricalKlinesConfig {
            data_root: temp.path().to_path_buf(),
            verify_checksum: false,
            ..HistoricalKlinesConfig::default()
        };
        let archives = plan_required_archives(&req);
        let archive = &archives[0];
        let zip_path = cfg.data_root.join(&archive.relative_path);
        write_zip(&zip_path, sample_csv());

        let fetcher = MockFetcher::default();
        let local = sync_archives_with_fetcher(&archives, &cfg, &fetcher).unwrap();

        assert_eq!(local.len(), 1);
        assert_eq!(local[0].source, LocalArchiveSource::Cached);
    }
}

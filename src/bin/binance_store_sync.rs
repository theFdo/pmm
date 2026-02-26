use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use chrono::{Datelike, Days, NaiveDate, TimeZone, Utc};
use pmm::{load_1s_klines, BinanceSymbol, HistoricalKlinesConfig, Kline1s, KlineLoadRequest};
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension};

const BINANCE_REST_KLINES_URL: &str = "https://api.binance.com/api/v3/klines";
const STEP_MS: i64 = 1_000;
const DAY_MS: i64 = 86_400_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start_date = parse_start_date();
    let start_ts = day_start_ts_ms(start_date);
    let now_ts = floor_to_second_ms(Utc::now().timestamp_millis());
    let today_start_ts = day_start_ts_ms(
        Utc.timestamp_millis_opt(now_ts)
            .single()
            .expect("valid now timestamp")
            .date_naive(),
    );

    if now_ts <= start_ts {
        return Err(format!(
            "invalid range: start={} end={} (now floored)",
            start_date,
            Utc.timestamp_millis_opt(now_ts)
                .single()
                .expect("valid now timestamp")
                .date_naive()
        )
        .into());
    }

    let data_root = std::env::var("PMM_BINANCE_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/binance"));
    let store_path = std::env::var("PMM_BINANCE_STORE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_root.join("klines_1s.sqlite"));

    let cfg = HistoricalKlinesConfig {
        data_root: data_root.clone(),
        verify_checksum: true,
        ..HistoricalKlinesConfig::default()
    };

    let mut store = KlineStore::open(&store_path)?;
    let rest_client = Client::builder()
        .timeout(Duration::from_millis(15_000))
        .build()?;

    let symbols = [
        BinanceSymbol::BtcUsdt,
        BinanceSymbol::EthUsdt,
        BinanceSymbol::SolUsdt,
        BinanceSymbol::XrpUsdt,
    ];

    println!(
        "Combined store sync start | store={} data_root={} start={} now_utc={}",
        store_path.display(),
        data_root.display(),
        start_date,
        Utc.timestamp_millis_opt(now_ts)
            .single()
            .expect("valid now timestamp")
            .format("%Y-%m-%d %H:%M:%S")
    );

    for symbol in symbols {
        sync_symbol(
            &mut store,
            &rest_client,
            symbol,
            start_ts,
            today_start_ts,
            now_ts,
            &cfg,
        )?;
    }

    println!("All symbols synced and completeness asserted.");
    Ok(())
}

fn parse_start_date() -> NaiveDate {
    if let Ok(raw) = std::env::var("PMM_KLINE_START_DATE") {
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .expect("PMM_KLINE_START_DATE must be YYYY-MM-DD")
    } else {
        NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid default start date")
    }
}

fn sync_symbol(
    store: &mut KlineStore,
    rest_client: &Client,
    symbol: BinanceSymbol,
    start_ts: i64,
    today_start_ts: i64,
    now_ts: i64,
    cfg: &HistoricalKlinesConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== {} ===", symbol.as_str());

    // Pass 1: fill by full-month archives where month coverage is incomplete.
    let mut month = Utc
        .timestamp_millis_opt(start_ts)
        .single()
        .expect("valid start timestamp")
        .date_naive();
    month = NaiveDate::from_ymd_opt(month.year(), month.month(), 1).expect("valid month start");

    while day_start_ts_ms(month) < today_start_ts {
        let next_month = next_month(month);
        let month_start_ts = std::cmp::max(start_ts, day_start_ts_ms(month));
        let month_end_ts = std::cmp::min(today_start_ts, day_start_ts_ms(next_month));
        if month_end_ts <= month_start_ts {
            break;
        }

        let expected = expected_points(month_start_ts, month_end_ts);
        let have = store.count_range(symbol, month_start_ts, month_end_ts)?;
        if have < expected {
            let req = KlineLoadRequest {
                symbol,
                start_ts_ms_utc: month_start_ts,
                end_ts_ms_utc_exclusive: month_end_ts,
            };
            let loaded = load_1s_klines(&req, cfg)?;
            store.upsert_rows(symbol, &loaded.rows)?;
            let after = store.count_range(symbol, month_start_ts, month_end_ts)?;
            println!(
                "month {} -> {} | expected={} before={} after={} missing_after={}",
                month,
                next_month,
                expected,
                have,
                after,
                expected.saturating_sub(after)
            );
        }

        month = next_month;
    }

    // Pass 2: day-level refill for any remaining day gaps.
    let mut day = Utc
        .timestamp_millis_opt(start_ts)
        .single()
        .expect("valid start timestamp")
        .date_naive();
    while day_start_ts_ms(day) < today_start_ts {
        let day_start = std::cmp::max(start_ts, day_start_ts_ms(day));
        let day_end = std::cmp::min(today_start_ts, day_start + DAY_MS);

        let expected = expected_points(day_start, day_end);
        let have = store.count_range(symbol, day_start, day_end)?;
        if have < expected {
            let req = KlineLoadRequest {
                symbol,
                start_ts_ms_utc: day_start,
                end_ts_ms_utc_exclusive: day_end,
            };
            let loaded = load_1s_klines(&req, cfg)?;
            store.upsert_rows(symbol, &loaded.rows)?;
            let after = store.count_range(symbol, day_start, day_end)?;
            println!(
                "day {} | expected={} before={} after={} missing_after={}",
                day,
                expected,
                have,
                after,
                expected.saturating_sub(after)
            );
        }

        day = day
            .checked_add_days(Days::new(1))
            .expect("next day should exist");
    }

    // Pass 3: REST fill for the tail from UTC day start to now.
    if now_ts > today_start_ts {
        let expected = expected_points(today_start_ts, now_ts);
        let have = store.count_range(symbol, today_start_ts, now_ts)?;
        if have < expected {
            fetch_rest_tail_and_upsert(store, rest_client, symbol, today_start_ts, now_ts)?;
            let after = store.count_range(symbol, today_start_ts, now_ts)?;
            println!(
                "rest tail {} -> now | expected={} before={} after={} missing_after={}",
                Utc.timestamp_millis_opt(today_start_ts)
                    .single()
                    .expect("valid day start")
                    .format("%Y-%m-%d %H:%M:%S"),
                expected,
                have,
                after,
                expected.saturating_sub(after)
            );
        }
    }

    // Final completeness assertion for full range.
    let expected_total = expected_points(start_ts, now_ts);
    let have_total = store.count_range(symbol, start_ts, now_ts)?;
    if have_total != expected_total {
        return Err(format!(
            "completeness assertion failed for {}: expected={} have={} missing={}",
            symbol.as_str(),
            expected_total,
            have_total,
            expected_total.saturating_sub(have_total)
        )
        .into());
    }

    println!(
        "COMPLETE {} | expected={} have={} missing=0",
        symbol.as_str(),
        expected_total,
        have_total
    );

    Ok(())
}

fn fetch_rest_tail_and_upsert(
    store: &mut KlineStore,
    client: &Client,
    symbol: BinanceSymbol,
    start_ts: i64,
    end_ts_exclusive: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cursor = start_ts;

    while cursor < end_ts_exclusive {
        let response = client
            .get(BINANCE_REST_KLINES_URL)
            .query(&[
                ("symbol", symbol.as_str()),
                ("interval", "1s"),
                ("startTime", &cursor.to_string()),
                ("endTime", &end_ts_exclusive.to_string()),
                ("limit", "1000"),
            ])
            .send()?;

        if !response.status().is_success() {
            return Err(format!(
                "binance REST error for {}: status={} start={} end={}",
                symbol.as_str(),
                response.status(),
                cursor,
                end_ts_exclusive
            )
            .into());
        }

        let payload: serde_json::Value = response.json()?;
        let rows = payload
            .as_array()
            .ok_or("unexpected REST payload: expected top-level array")?;

        if rows.is_empty() {
            break;
        }

        let mut batch = Vec::with_capacity(rows.len());
        for row in rows {
            batch.push(parse_rest_kline_row(row)?);
        }

        let last_open = batch
            .last()
            .map(|row| row.open_time_ms)
            .ok_or("empty REST batch after parse")?;
        store.upsert_rows(symbol, &batch)?;

        let next_cursor = last_open.saturating_add(STEP_MS);
        if next_cursor <= cursor {
            return Err(format!(
                "rest cursor did not advance for {} at {}",
                symbol.as_str(),
                cursor
            )
            .into());
        }

        cursor = std::cmp::min(next_cursor, end_ts_exclusive);
        sleep(Duration::from_millis(25));
    }

    Ok(())
}

fn parse_rest_kline_row(value: &serde_json::Value) -> Result<Kline1s, Box<dyn std::error::Error>> {
    let row = value
        .as_array()
        .ok_or("unexpected REST row: expected array")?;
    if row.len() < 11 {
        return Err(format!("unexpected REST row length: {}", row.len()).into());
    }

    let open_time_ms = json_i64(&row[0])?;
    let close_time_ms = json_i64(&row[6])?;

    Ok(Kline1s {
        open_time_ms,
        open: json_f64(&row[1])?,
        high: json_f64(&row[2])?,
        low: json_f64(&row[3])?,
        close: json_f64(&row[4])?,
        volume: json_f64(&row[5])?,
        close_time_ms,
        quote_asset_volume: json_f64(&row[7])?,
        trade_count: json_u64(&row[8])?,
        taker_buy_base_volume: json_f64(&row[9])?,
        taker_buy_quote_volume: json_f64(&row[10])?,
    })
}

fn json_i64(value: &serde_json::Value) -> Result<i64, Box<dyn std::error::Error>> {
    if let Some(v) = value.as_i64() {
        return Ok(v);
    }
    let text = value.as_str().ok_or("expected i64-compatible value")?;
    Ok(text.parse()?)
}

fn json_u64(value: &serde_json::Value) -> Result<u64, Box<dyn std::error::Error>> {
    if let Some(v) = value.as_u64() {
        return Ok(v);
    }
    let text = value.as_str().ok_or("expected u64-compatible value")?;
    Ok(text.parse()?)
}

fn json_f64(value: &serde_json::Value) -> Result<f64, Box<dyn std::error::Error>> {
    if let Some(v) = value.as_f64() {
        return Ok(v);
    }
    let text = value.as_str().ok_or("expected f64-compatible value")?;
    Ok(text.parse()?)
}

struct KlineStore {
    conn: Connection,
}

impl KlineStore {
    fn open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            ",
        )?;
        ensure_compact_schema(&conn)?;

        Ok(Self { conn })
    }

    fn upsert_rows(
        &mut self,
        symbol: BinanceSymbol,
        rows: &[Kline1s],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if rows.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
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
                ON CONFLICT(symbol_id, open_time_ms) DO UPDATE SET
                    open = excluded.open,
                    high = excluded.high,
                    low = excluded.low,
                    close = excluded.close,
                    volume = excluded.volume,
                    close_time_ms = excluded.close_time_ms,
                    quote_asset_volume = excluded.quote_asset_volume,
                    trade_count = excluded.trade_count,
                    taker_buy_base_volume = excluded.taker_buy_base_volume,
                    taker_buy_quote_volume = excluded.taker_buy_quote_volume
                ",
            )?;

            for row in rows {
                stmt.execute(params![
                    symbol_id(symbol),
                    row.open_time_ms,
                    row.open,
                    row.high,
                    row.low,
                    row.close,
                    row.volume,
                    row.close_time_ms,
                    row.quote_asset_volume,
                    row.trade_count,
                    row.taker_buy_base_volume,
                    row.taker_buy_quote_volume,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    fn count_range(
        &self,
        symbol: BinanceSymbol,
        start_ts_ms: i64,
        end_ts_ms_exclusive: i64,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let count: i64 = self.conn.query_row(
            "
            SELECT COUNT(*)
            FROM klines_1s
            WHERE symbol_id = ?1
              AND open_time_ms >= ?2
              AND open_time_ms < ?3
            ",
            params![symbol_id(symbol), start_ts_ms, end_ts_ms_exclusive],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }
}

fn ensure_compact_schema(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    if !table_exists(conn, "klines_1s")? {
        create_compact_table(conn)?;
        return Ok(());
    }

    let has_symbol_id = table_has_column(conn, "klines_1s", "symbol_id")?;
    let has_symbol = table_has_column(conn, "klines_1s", "symbol")?;
    let without_rowid = table_is_without_rowid(conn, "klines_1s")?;

    if has_symbol_id && without_rowid {
        return Ok(());
    }

    println!("Schema migration: converting klines_1s to compact symbol_id + WITHOUT ROWID...");
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let migrate_result = (|| -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(
            "
            CREATE TABLE klines_1s_new (
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
        )?;

        if has_symbol {
            conn.execute_batch(
                "
                INSERT INTO klines_1s_new (
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
                )
                SELECT
                    CASE symbol
                        WHEN 'BTCUSDT' THEN 1
                        WHEN 'ETHUSDT' THEN 2
                        WHEN 'SOLUSDT' THEN 3
                        WHEN 'XRPUSDT' THEN 4
                    END AS symbol_id,
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
                FROM klines_1s;
                ",
            )?;
        } else if has_symbol_id {
            conn.execute_batch(
                "
                INSERT INTO klines_1s_new (
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
                )
                SELECT
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
                FROM klines_1s;
                ",
            )?;
        } else {
            return Err("unsupported existing klines_1s schema (missing symbol/symbol_id)".into());
        }

        conn.execute_batch(
            "
            DROP TABLE klines_1s;
            ALTER TABLE klines_1s_new RENAME TO klines_1s;
            ",
        )?;
        Ok(())
    })();

    match migrate_result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")?;
            println!("Schema migration complete. Running VACUUM to reclaim space...");
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
}

fn create_compact_table(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
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
    )?;
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
            params![table],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

fn table_has_column(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_is_without_rowid(
    conn: &Connection,
    table: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            params![table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(sql
        .as_deref()
        .map(|ddl| ddl.to_ascii_uppercase().contains("WITHOUT ROWID"))
        .unwrap_or(false))
}

fn symbol_id(symbol: BinanceSymbol) -> i64 {
    match symbol {
        BinanceSymbol::BtcUsdt => 1,
        BinanceSymbol::EthUsdt => 2,
        BinanceSymbol::SolUsdt => 3,
        BinanceSymbol::XrpUsdt => 4,
    }
}

fn expected_points(start_ts: i64, end_ts_exclusive: i64) -> u64 {
    if end_ts_exclusive <= start_ts {
        0
    } else {
        ((end_ts_exclusive - start_ts) / STEP_MS) as u64
    }
}

fn day_start_ts_ms(date: NaiveDate) -> i64 {
    Utc.with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .expect("valid day start")
        .timestamp_millis()
}

fn next_month(date: NaiveDate) -> NaiveDate {
    if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1).expect("valid next month")
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1).expect("valid next month")
    }
}

fn floor_to_second_ms(ts_ms: i64) -> i64 {
    ts_ms.div_euclid(STEP_MS) * STEP_MS
}

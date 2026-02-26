//! Step 4 dashboard logic: filters, in-interval evaluation, formatting, and realtime rendering.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use chrono::{Datelike, Days, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;
use serde::{Deserialize, Deserializer, Serialize};

use crate::discovery::{
    build_previous_active_and_next_discovery_keys, DiscoveryWindow, ALL_COINS, ALL_DURATIONS,
};
#[cfg(feature = "discovery-sdk")]
use crate::discovery::{
    resolve_discovery_batch, DiscoveryConfig, DiscoveryRow, DiscoveryStatus, ScheduledDiscoveryKey,
    SdkMarket, UnresolvedReason,
};
use crate::slug::{Coin, Duration, SlugConfig};

pub const DASHBOARD_HEADERS: [&str; 21] = [
    "Link",
    "Coin",
    "Duration",
    "Bets Open",
    "In Interval",
    "End",
    "Ref Price",
    "Price",
    "Probability",
    "Best Bid YES",
    "Best Ask YES",
    "Position Net",
    "Pos YES",
    "Pos NO",
    "Offer YES",
    "Offer NO",
    "Net Profit",
    "Taker Fee %",
    "Maker Fee %",
    "Fee Exp",
    "Reward %",
];

pub const DASHBOARD_COLUMN_KEYS: [&str; 21] = [
    "link",
    "coin",
    "duration",
    "bets_open",
    "in_interval",
    "end",
    "ref_price",
    "price",
    "probability",
    "best_bid_yes",
    "best_ask_yes",
    "position_net",
    "pos_yes",
    "pos_no",
    "offer_yes",
    "offer_no",
    "net_profit",
    "taker_fee_pct",
    "maker_fee_pct",
    "fee_exponent",
    "reward_pct",
];

const COIN_OPTIONS: [&str; 4] = ["BTC", "ETH", "SOL", "XRP"];
const DURATION_OPTIONS: [&str; 5] = ["5m", "15m", "1h", "4h", "1d"];
const DASHBOARD_CLIENT_SCRIPT: &str = r#"<script>
(function () {
  const params = window.location.search;
  const tbody = document.getElementById('dashboard-body');
  const rowCount = document.getElementById('row-count');
  const filterForm = document.getElementById('filters-form');
  let inflight = false;

  function esc(v) {
    return String(v)
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;')
      .replaceAll('"', '&quot;')
      .replaceAll("'", '&#39;');
  }

  function localHHMM(ts) {
    if (!Number.isFinite(ts) || ts <= 0) {
      return '-';
    }
    const d = new Date(ts * 1000);
    const h = String(d.getHours()).padStart(2, '0');
    const m = String(d.getMinutes()).padStart(2, '0');
    return h + ':' + m;
  }

  function hasMock(row, key) {
    return Array.isArray(row.mock_columns) && row.mock_columns.includes(key);
  }

  function tdClass(row, key, extra) {
    const cls = [];
    if (extra) {
      cls.push(extra);
    }
    if (hasMock(row, key)) {
      cls.push('cell-mock');
    }
    return cls.join(' ');
  }

  function renderRow(row, idx) {
    const endLocal = localHHMM(Number(row.end_ts_utc));
    return `<tr data-row="${idx}">
      <td class="${tdClass(row, 'link', 'market-cell')}">
        <a class="market-btn" target="_blank" rel="noopener noreferrer" href="${esc(row.link_url)}">Open Market</a>
        <span class="slug-id" title="${esc(row.slug)}">${esc(row.slug)}</span>
      </td>
      <td class="${tdClass(row, 'coin', '')}">${esc(row.coin)}</td>
      <td class="${tdClass(row, 'duration', '')}">${esc(row.duration)}</td>
      <td class="${tdClass(row, 'bets_open', '')}">${esc(row.bets_open)}</td>
      <td class="${tdClass(row, 'in_interval', '')}">${esc(row.in_interval)}</td>
      <td data-end-ts="${row.end_ts_utc}" class="${tdClass(row, 'end', '')}">${esc(endLocal)}</td>
      <td class="${tdClass(row, 'ref_price', '')}">${esc(row.ref_price)}</td>
      <td class="${tdClass(row, 'price', '')}">${esc(row.price)}</td>
      <td class="${tdClass(row, 'probability', '')}">${esc(row.probability)}</td>
      <td class="${tdClass(row, 'best_bid_yes', '')}">${esc(row.best_bid_yes)}</td>
      <td class="${tdClass(row, 'best_ask_yes', '')}">${esc(row.best_ask_yes)}</td>
      <td class="${tdClass(row, 'position_net', '')}">${esc(row.position_net)}</td>
      <td class="${tdClass(row, 'pos_yes', '')}">${esc(row.pos_yes)}</td>
      <td class="${tdClass(row, 'pos_no', '')}">${esc(row.pos_no)}</td>
      <td class="${tdClass(row, 'offer_yes', '')}">${esc(row.offer_yes)}</td>
      <td class="${tdClass(row, 'offer_no', '')}">${esc(row.offer_no)}</td>
      <td class="${tdClass(row, 'net_profit', '')}">${esc(row.net_profit)}</td>
      <td class="${tdClass(row, 'taker_fee_pct', '')}">${esc(row.taker_fee_pct)}</td>
      <td class="${tdClass(row, 'maker_fee_pct', '')}">${esc(row.maker_fee_pct)}</td>
      <td class="${tdClass(row, 'fee_exponent', '')}">${esc(row.fee_exponent)}</td>
      <td class="${tdClass(row, 'reward_pct', '')}">${esc(row.reward_pct)}</td>
    </tr>`;
  }

  function rewriteExistingEndCells() {
    document.querySelectorAll('[data-end-ts]').forEach((td) => {
      const ts = Number(td.getAttribute('data-end-ts'));
      td.textContent = localHHMM(ts);
    });
  }

  async function refresh() {
    if (inflight) {
      return;
    }
    inflight = true;
    try {
      const r = await fetch('/dashboard/snapshot' + params, { cache: 'no-store' });
      if (!r.ok) {
        return;
      }
      const payload = await r.json();
      const rows = Array.isArray(payload.rows) ? payload.rows : [];
      tbody.innerHTML = rows.map((row, idx) => renderRow(row, idx)).join('');
      if (rowCount) {
        rowCount.textContent = String(rows.length);
      }
    } catch (_err) {
      // Keep UI stale on transient polling/network failures.
    } finally {
      inflight = false;
    }
  }

  rewriteExistingEndCells();
  if (filterForm) {
    filterForm.addEventListener('change', () => {
      const next = new URLSearchParams(new FormData(filterForm)).toString();
      window.location.assign(next ? `/dashboard?${next}` : '/dashboard');
    });
  }
  setInterval(refresh, 250);
})();
</script>"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub rows: Vec<DashboardRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardRow {
    pub slug: String,
    pub coin: String,
    pub duration: String,
    pub start_ts_utc: i64,
    pub end_ts_utc: i64,
    pub bets_open: Option<String>,
    pub in_interval: Option<String>,
    pub end_hhmm: Option<String>,
    pub ref_price: Option<String>,
    pub price: Option<String>,
    pub probability: Option<String>,
    pub best_bid_yes: Option<String>,
    pub best_ask_yes: Option<String>,
    pub position_net: Option<String>,
    pub pos_yes: Option<String>,
    pub pos_no: Option<String>,
    pub offer_yes: Option<String>,
    pub offer_no: Option<String>,
    pub net_profit: Option<String>,
    pub taker_fee_pct: Option<String>,
    pub maker_fee_pct: Option<String>,
    pub fee_exponent: Option<String>,
    pub reward_pct: Option<String>,
    pub mock_columns: Vec<String>,
}

impl DashboardRow {
    pub fn unresolved(
        slug: impl Into<String>,
        coin: impl Into<String>,
        duration: impl Into<String>,
    ) -> Self {
        Self::unresolved_with_times(slug, coin, duration, 0, 0)
    }

    pub fn unresolved_with_times(
        slug: impl Into<String>,
        coin: impl Into<String>,
        duration: impl Into<String>,
        start_ts_utc: i64,
        end_ts_utc: i64,
    ) -> Self {
        Self {
            slug: slug.into(),
            coin: coin.into(),
            duration: duration.into(),
            start_ts_utc,
            end_ts_utc,
            bets_open: None,
            in_interval: None,
            end_hhmm: None,
            ref_price: None,
            price: None,
            probability: None,
            best_bid_yes: None,
            best_ask_yes: None,
            position_net: None,
            pos_yes: None,
            pos_no: None,
            offer_yes: None,
            offer_no: None,
            net_profit: None,
            taker_fee_pct: None,
            maker_fee_pct: None,
            fee_exponent: None,
            reward_pct: None,
            mock_columns: default_mock_columns(),
        }
    }

    pub fn is_mock_column(&self, column_key: &str) -> bool {
        self.mock_columns.iter().any(|entry| entry == column_key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardDisplaySnapshot {
    pub now_ts_utc: i64,
    pub rows: Vec<DashboardDisplayRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardDisplayRow {
    pub slug: String,
    pub link_url: String,
    pub coin: String,
    pub duration: String,
    pub start_ts_utc: i64,
    pub end_ts_utc: i64,
    pub bets_open: String,
    pub in_interval: String,
    pub end_hhmm: String,
    pub ref_price: String,
    pub price: String,
    pub probability: String,
    pub best_bid_yes: String,
    pub best_ask_yes: String,
    pub position_net: String,
    pub pos_yes: String,
    pub pos_no: String,
    pub offer_yes: String,
    pub offer_no: String,
    pub net_profit: String,
    pub taker_fee_pct: String,
    pub maker_fee_pct: String,
    pub fee_exponent: String,
    pub reward_pct: String,
    pub mock_columns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DashboardQuery {
    #[serde(default, deserialize_with = "deserialize_vec_or_single")]
    pub coin: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_vec_or_single")]
    pub duration: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_vec_or_single")]
    pub bets_open: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_vec_or_single")]
    pub in_interval: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum VecOrSingle {
    One(String),
    Many(Vec<String>),
}

fn deserialize_vec_or_single<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<VecOrSingle>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(VecOrSingle::One(value)) => Ok(vec![value]),
        Some(VecOrSingle::Many(values)) => Ok(values),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BetsOpenFilter {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InIntervalFilter {
    Yes,
    No,
}

#[derive(Debug, Clone)]
pub struct DashboardFilters {
    pub coins: HashSet<String>,
    pub durations: HashSet<String>,
    pub bets_open: HashSet<BetsOpenFilter>,
    pub in_interval: HashSet<InIntervalFilter>,
}

impl DashboardFilters {
    pub fn from_query(query: &DashboardQuery) -> Self {
        Self {
            coins: parse_set_or_all(&query.coin, &COIN_OPTIONS),
            durations: parse_set_or_all(&query.duration, &DURATION_OPTIONS),
            bets_open: parse_bets_open(&query.bets_open),
            in_interval: parse_in_interval(&query.in_interval),
        }
    }

    pub fn all_selected() -> Self {
        Self::from_query(&DashboardQuery::default())
    }

    pub fn coin_selected(&self, coin: &str) -> bool {
        self.coins.contains(coin)
    }

    pub fn duration_selected(&self, duration: &str) -> bool {
        self.durations.contains(duration)
    }

    pub fn bets_open_selected(&self, value: BetsOpenFilter) -> bool {
        self.bets_open.contains(&value)
    }

    pub fn in_interval_selected(&self, value: InIntervalFilter) -> bool {
        self.in_interval.contains(&value)
    }

    fn allows_unknown_bets_open(&self) -> bool {
        self.bets_open_selected(BetsOpenFilter::Open)
            && self.bets_open_selected(BetsOpenFilter::Closed)
    }
}

pub trait DashboardSnapshotSource: Send + Sync + 'static {
    fn snapshot(&self) -> DashboardSnapshot;
}

#[derive(Clone)]
pub struct InMemoryMockSnapshotSource {
    inner: Arc<RwLock<DashboardSnapshot>>,
}

impl InMemoryMockSnapshotSource {
    pub fn new(snapshot: DashboardSnapshot) -> Self {
        Self {
            inner: Arc::new(RwLock::new(snapshot)),
        }
    }

    pub fn demo() -> Self {
        Self::new(demo_snapshot())
    }

    pub fn replace_snapshot(&self, snapshot: DashboardSnapshot) {
        let mut guard = self
            .inner
            .write()
            .expect("in-memory snapshot lock should not be poisoned");
        *guard = snapshot;
    }
}

impl DashboardSnapshotSource for InMemoryMockSnapshotSource {
    fn snapshot(&self) -> DashboardSnapshot {
        self.inner
            .read()
            .expect("in-memory snapshot lock should not be poisoned")
            .clone()
    }
}

#[cfg(feature = "discovery-sdk")]
#[derive(Debug, Clone, Copy)]
pub struct LiveDiscoveryConfig {
    pub refresh_interval_ms: u64,
    pub slug_config: SlugConfig,
    pub discovery_config: DiscoveryConfig,
}

#[cfg(feature = "discovery-sdk")]
impl Default for LiveDiscoveryConfig {
    fn default() -> Self {
        let offset = std::env::var("PMFLIPS_DISCOVERY_OFFSET_4H_MIN")
            .ok()
            .and_then(|raw| raw.parse::<i32>().ok())
            .unwrap_or(60);

        let timeout_ms = std::env::var("PMM_DISCOVERY_TIMEOUT_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(3_000);
        let batch_size = std::env::var("PMM_DISCOVERY_BATCH_SIZE")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(64);
        let max_retries = std::env::var("PMM_DISCOVERY_MAX_RETRIES")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(2);
        let retry_backoff_ms = std::env::var("PMM_DISCOVERY_RETRY_BACKOFF_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(200);
        let refresh_interval_ms = std::env::var("PMM_DASHBOARD_DISCOVERY_REFRESH_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(1_000);

        Self {
            refresh_interval_ms,
            slug_config: SlugConfig {
                discovery_offset_4h_min: offset,
            },
            discovery_config: DiscoveryConfig {
                timeout_ms,
                batch_size,
                max_retries,
                retry_backoff_ms,
                include_tag: false,
            },
        }
    }
}

#[cfg(feature = "discovery-sdk")]
#[derive(Clone)]
pub struct LiveDiscoverySnapshotSource {
    inner: Arc<RwLock<DashboardSnapshot>>,
}

#[cfg(feature = "discovery-sdk")]
impl LiveDiscoverySnapshotSource {
    pub fn spawn(config: LiveDiscoveryConfig) -> Self {
        let initial = demo_snapshot();
        let inner = Arc::new(RwLock::new(initial));
        let inner_bg = Arc::clone(&inner);

        tokio::spawn(async move {
            loop {
                let refreshed = build_live_discovery_snapshot(config).await;
                {
                    let mut guard = inner_bg
                        .write()
                        .expect("live discovery snapshot lock should not be poisoned");
                    *guard = refreshed;
                }
                tokio::time::sleep(std::time::Duration::from_millis(config.refresh_interval_ms))
                    .await;
            }
        });

        Self { inner }
    }
}

#[cfg(feature = "discovery-sdk")]
impl DashboardSnapshotSource for LiveDiscoverySnapshotSource {
    fn snapshot(&self) -> DashboardSnapshot {
        self.inner
            .read()
            .expect("live discovery snapshot lock should not be poisoned")
            .clone()
    }
}

pub fn dashboard_router(source: Arc<dyn DashboardSnapshotSource>) -> Router {
    Router::new()
        .route("/dashboard", get(get_dashboard_html))
        .route("/dashboard/snapshot", get(get_dashboard_snapshot))
        .with_state(DashboardAppState { source })
}

pub fn market_link(slug: &str) -> String {
    format!("https://polymarket.com/event/{slug}")
}

pub fn compute_in_interval(now_ts_utc: i64, start_ts_utc: i64, end_ts_utc: i64) -> bool {
    start_ts_utc <= now_ts_utc && now_ts_utc < end_ts_utc
}

pub fn apply_filters(
    rows: &[DashboardRow],
    filters: &DashboardFilters,
    now_ts_utc: i64,
) -> Vec<DashboardRow> {
    rows.iter()
        .filter(|row| filters.coin_selected(&row.coin))
        .filter(|row| filters.duration_selected(&row.duration))
        .filter(|row| row_matches_bets_open(row, filters))
        .filter(|row| row_matches_in_interval(row, filters, now_ts_utc))
        .cloned()
        .collect()
}

pub fn format_row_for_display(row: &DashboardRow, now_ts_utc: i64) -> DashboardDisplayRow {
    let in_interval = compute_in_interval(now_ts_utc, row.start_ts_utc, row.end_ts_utc);

    DashboardDisplayRow {
        slug: row.slug.clone(),
        link_url: market_link(&row.slug),
        coin: row.coin.clone(),
        duration: row.duration.clone(),
        start_ts_utc: row.start_ts_utc,
        end_ts_utc: row.end_ts_utc,
        bets_open: format_column_value("bets_open", row.bets_open.as_deref()),
        in_interval: if in_interval { "yes" } else { "no" }.to_string(),
        end_hhmm: utc_hhmm(row.end_ts_utc),
        ref_price: format_column_value("ref_price", row.ref_price.as_deref()),
        price: format_column_value("price", row.price.as_deref()),
        probability: format_column_value("probability", row.probability.as_deref()),
        best_bid_yes: format_column_value("best_bid_yes", row.best_bid_yes.as_deref()),
        best_ask_yes: format_column_value("best_ask_yes", row.best_ask_yes.as_deref()),
        position_net: format_column_value("position_net", row.position_net.as_deref()),
        pos_yes: format_column_value("pos_yes", row.pos_yes.as_deref()),
        pos_no: format_column_value("pos_no", row.pos_no.as_deref()),
        offer_yes: format_column_value("offer_yes", row.offer_yes.as_deref()),
        offer_no: format_column_value("offer_no", row.offer_no.as_deref()),
        net_profit: format_column_value("net_profit", row.net_profit.as_deref()),
        taker_fee_pct: format_column_value("taker_fee_pct", row.taker_fee_pct.as_deref()),
        maker_fee_pct: format_column_value("maker_fee_pct", row.maker_fee_pct.as_deref()),
        fee_exponent: format_column_value("fee_exponent", row.fee_exponent.as_deref()),
        reward_pct: format_column_value("reward_pct", row.reward_pct.as_deref()),
        mock_columns: row.mock_columns.clone(),
    }
}

pub fn build_display_snapshot(
    snapshot: &DashboardSnapshot,
    filters: &DashboardFilters,
    now_ts_utc: i64,
) -> DashboardDisplaySnapshot {
    let filtered = apply_filters(&snapshot.rows, filters, now_ts_utc);
    let rows = filtered
        .iter()
        .map(|row| format_row_for_display(row, now_ts_utc))
        .collect();

    DashboardDisplaySnapshot { now_ts_utc, rows }
}

pub fn render_dashboard_html(snapshot: &DashboardSnapshot) -> String {
    let filters = DashboardFilters::all_selected();
    render_dashboard_html_with_filters(snapshot, &filters, Utc::now().timestamp())
}

fn render_dashboard_html_with_filters(
    snapshot: &DashboardSnapshot,
    filters: &DashboardFilters,
    now_ts_utc: i64,
) -> String {
    let display = build_display_snapshot(snapshot, filters, now_ts_utc);
    let now_utc = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let mut out = String::new();
    out.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>PMM Dashboard</title>\n");
    out.push_str("<style>:root{--bg:#f5f1e7;--bg2:#e9f0f2;--card:#ffffff;--ink:#182026;--muted:#5f6a73;--line:#d7dce1;--head:#14343f;--btn:#0c5f78;--btnhover:#094d61;--mockbg:#fff5b8;--mockink:#555c63}*{box-sizing:border-box}body{margin:0;color:var(--ink);font-family:\"Space Grotesk\",\"Avenir Next\",\"Segoe UI\",sans-serif;background:radial-gradient(circle at 10% 5%, #ffe7a3 0%, transparent 30%),radial-gradient(circle at 90% 0%, #b9e5f0 0%, transparent 28%),linear-gradient(160deg,var(--bg),var(--bg2));min-height:100vh}.shell{max-width:none;width:100%;margin:0;padding:20px 16px 26px}.hero{background:linear-gradient(135deg,#102f3a 0%,#24576b 100%);color:#f7fbfc;border-radius:16px;padding:18px 20px;box-shadow:0 10px 30px rgba(16,47,58,.25)}.hero h1{margin:0 0 8px;font-size:1.58rem}.hero-meta{display:flex;gap:14px;flex-wrap:wrap;font-size:.9rem;color:#dcebf0}.filters{margin-top:12px;background:rgba(255,255,255,.1);border:1px solid rgba(255,255,255,.22);border-radius:12px;padding:10px 12px}.filter-grid{display:grid;grid-template-columns:repeat(4,minmax(160px,1fr));gap:10px}.filter-block{background:rgba(0,0,0,.12);border-radius:10px;padding:8px}.filter-title{font-size:.74rem;letter-spacing:.04em;text-transform:uppercase;margin:0 0 6px;color:#dbeaf0}.filter-item{display:flex;align-items:center;gap:6px;font-size:.85rem;margin:3px 0}.filter-actions{margin-top:10px;display:flex;gap:10px;align-items:center}.auto-note{font-size:.76rem;color:#dcebf0;opacity:.9}.btn{padding:7px 10px;border-radius:8px;border:1px solid rgba(0,0,0,.15);font-weight:700;font-size:.78rem;cursor:pointer}.btn-reset{background:#e4eef2;color:#1b3642;text-decoration:none}.card{margin-top:14px;background:var(--card);border:1px solid #cbd4db;border-radius:16px;overflow:hidden;box-shadow:0 12px 28px rgba(26,35,42,.12)}.table-wrap{overflow:auto;max-height:75vh}table{width:100%;border-collapse:collapse;min-width:1300px}thead th{position:sticky;top:0;z-index:2;background:var(--head);color:#f2f7f9;font-size:.79rem;text-transform:uppercase;letter-spacing:.04em;padding:10px;border-bottom:1px solid #0e2730}tbody td{font-size:.84rem;padding:8px 10px;border-bottom:1px solid var(--line);white-space:nowrap}tbody tr:nth-child(even){background:#fafcfd}.market-cell{min-width:220px}.market-btn{display:inline-flex;align-items:center;justify-content:center;background:linear-gradient(135deg,var(--btn),#0f7592);color:#fff;text-decoration:none;padding:7px 10px;border-radius:9px;font-weight:700;font-size:.76rem;border:1px solid rgba(0,0,0,.12);box-shadow:0 2px 8px rgba(12,95,120,.25)}.market-btn:hover{background:linear-gradient(135deg,var(--btnhover),#0d5f78)}.slug-id{display:block;margin-top:6px;font-family:\"IBM Plex Mono\",\"SFMono-Regular\",monospace;font-size:.67rem;color:var(--muted);max-width:260px;overflow:hidden;text-overflow:ellipsis}.cell-mock{background:linear-gradient(135deg,var(--mockbg) 0%,#fff3ca 100%);color:var(--mockink)}.cell-mock::after{content:\" M\";font-size:.62rem;font-weight:700;color:#8c6a00}.legend{padding:10px 14px;border-top:1px solid var(--line);font-size:.8rem;color:var(--muted);background:#f8fbfc;display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap}.legend b{color:#8c6a00}@media (max-width:980px){.filter-grid{grid-template-columns:repeat(2,minmax(150px,1fr))}}@media (max-width:760px){.hero h1{font-size:1.28rem}.shell{padding:12px}.card{margin-top:12px;border-radius:12px}.filter-grid{grid-template-columns:1fr}}</style>\n");
    out.push_str("</head><body><main class=\"shell\">\n");
    out.push_str("<section class=\"hero\"><h1>PMM Dashboard</h1>");
    out.push_str("<div class=\"hero-meta\">\n");
    out.push_str("<span>Scope: 4 coins × 5 durations × previous/active/next</span>");
    out.push_str(&format!(
        "<span>Rows: <b id=\"row-count\">{}</b></span>",
        display.rows.len()
    ));
    out.push_str(&format!(
        "<span>Server UTC: {}</span>",
        escape_html(&now_utc)
    ));
    out.push_str("<span>Refresh: 250ms</span>");
    out.push_str("</div>");

    out.push_str(
        "<form id=\"filters-form\" class=\"filters\" method=\"get\" action=\"/dashboard\">\n",
    );
    out.push_str("<div class=\"filter-grid\">\n");
    out.push_str(&render_checkbox_group("Coin", "coin", &COIN_OPTIONS, |v| {
        filters.coin_selected(v)
    }));
    out.push_str(&render_checkbox_group(
        "Duration",
        "duration",
        &DURATION_OPTIONS,
        |v| filters.duration_selected(v),
    ));
    out.push_str(&render_checkbox_group(
        "Bets Open",
        "bets_open",
        &["open", "closed"],
        |v| match v {
            "open" => filters.bets_open_selected(BetsOpenFilter::Open),
            "closed" => filters.bets_open_selected(BetsOpenFilter::Closed),
            _ => false,
        },
    ));
    out.push_str(&render_checkbox_group(
        "In Interval",
        "in_interval",
        &["yes", "no"],
        |v| match v {
            "yes" => filters.in_interval_selected(InIntervalFilter::Yes),
            "no" => filters.in_interval_selected(InIntervalFilter::No),
            _ => false,
        },
    ));
    out.push_str("</div>");
    out.push_str("<div class=\"filter-actions\"><a class=\"btn btn-reset\" href=\"/dashboard\">Reset</a><span class=\"auto-note\">Auto-applies on checkbox change</span></div>");
    out.push_str("</form></section>\n");

    out.push_str(
        "<section class=\"card\"><div class=\"table-wrap\"><table id=\"dashboard-table\">\n",
    );
    out.push_str("<thead><tr>");
    for header in DASHBOARD_HEADERS {
        out.push_str("<th>");
        out.push_str(&escape_html(header));
        out.push_str("</th>");
    }
    out.push_str("</tr></thead><tbody id=\"dashboard-body\">\n");
    out.push_str(&render_rows_html(&display.rows));
    out.push_str("</tbody></table></div>");
    out.push_str("<div class=\"legend\"><span>Mock-backed cells are highlighted <b>yellow/grey</b> and tagged with <b>M</b>.</span><span>End is shown in <b>local browser time</b>.</span></div></section>");

    out.push_str(DASHBOARD_CLIENT_SCRIPT);

    out.push_str("</main></body></html>\n");
    out
}

#[cfg(feature = "discovery-sdk")]
async fn build_live_discovery_snapshot(config: LiveDiscoveryConfig) -> DashboardSnapshot {
    let now_ts = Utc::now().timestamp();
    let scheduled = match build_previous_active_and_next_discovery_keys(
        now_ts,
        &ALL_COINS,
        &ALL_DURATIONS,
        config.slug_config,
    ) {
        Ok(keys) => keys,
        Err(err) => {
            eprintln!("live discovery: failed to build scheduled keys: {err}");
            return DashboardSnapshot { rows: Vec::new() };
        }
    };

    let keys: Vec<_> = scheduled.iter().map(|entry| entry.key.clone()).collect();
    let rows = match resolve_discovery_batch(&keys, &config.discovery_config).await {
        Ok(resolved) => resolved
            .iter()
            .zip(scheduled.iter())
            .map(|(row, scheduled_key)| discovery_row_to_dashboard_row(row, scheduled_key))
            .collect(),
        Err(err) => {
            eprintln!("live discovery: batch resolution error: {err}");
            scheduled
                .iter()
                .map(|scheduled_key| {
                    unresolved_dashboard_row_with_reason(scheduled_key, &err.to_string())
                })
                .collect()
        }
    };

    DashboardSnapshot { rows }
}

#[cfg(feature = "discovery-sdk")]
fn discovery_row_to_dashboard_row(
    row: &DiscoveryRow<SdkMarket>,
    scheduled: &ScheduledDiscoveryKey,
) -> DashboardRow {
    let start_ts_utc = row.key.start_ts_utc;
    let end_ts_utc = interval_end_ts_utc(start_ts_utc, row.key.duration);
    let mut dashboard_row = DashboardRow::unresolved_with_times(
        row.key.slug.clone(),
        coin_label(row.key.coin),
        duration_label(row.key.duration),
        start_ts_utc,
        end_ts_utc,
    );

    dashboard_row.in_interval = Some(
        match scheduled.window {
            DiscoveryWindow::Active => "yes",
            DiscoveryWindow::Previous | DiscoveryWindow::Next => "no",
        }
        .to_string(),
    );
    dashboard_row.end_hhmm = Some(utc_hhmm(end_ts_utc));

    match &row.status {
        DiscoveryStatus::Resolved { market } => {
            if let Some(slug) = market.slug.clone() {
                dashboard_row.slug = slug;
            }
            dashboard_row.bets_open = market_bets_open_label(market);
            let fee_params = market_fee_params(market, row.key.duration);
            dashboard_row.taker_fee_pct = Some(fee_params.taker_fee_pct);
            dashboard_row.maker_fee_pct = Some(fee_params.maker_fee_pct);
            dashboard_row.fee_exponent = Some(fee_params.fee_exponent);
            dashboard_row.reward_pct =
                Some(market_reward_pct(market).unwrap_or_else(|| "0".to_string()));
            dashboard_row.mock_columns = resolved_mock_columns(&dashboard_row);
        }
        DiscoveryStatus::Unresolved { reason } => {
            if let UnresolvedReason::TransportError(message) = reason {
                dashboard_row.net_profit = Some(format!("transport:{message}"));
            }
        }
    }

    dashboard_row
}

#[cfg(feature = "discovery-sdk")]
fn unresolved_dashboard_row_with_reason(
    scheduled: &ScheduledDiscoveryKey,
    reason: &str,
) -> DashboardRow {
    let start_ts_utc = scheduled.key.start_ts_utc;
    let end_ts_utc = interval_end_ts_utc(start_ts_utc, scheduled.key.duration);
    let mut row = DashboardRow::unresolved_with_times(
        scheduled.key.slug.clone(),
        coin_label(scheduled.key.coin),
        duration_label(scheduled.key.duration),
        start_ts_utc,
        end_ts_utc,
    );

    row.in_interval = Some(
        match scheduled.window {
            DiscoveryWindow::Active => "yes",
            DiscoveryWindow::Previous | DiscoveryWindow::Next => "no",
        }
        .to_string(),
    );
    row.end_hhmm = Some(utc_hhmm(end_ts_utc));
    row.net_profit = Some(format!("transport:{reason}"));
    row
}

#[cfg(feature = "discovery-sdk")]
fn resolved_mock_columns(row: &DashboardRow) -> Vec<String> {
    let mut mock = default_mock_columns();
    let live_columns = [
        ("bets_open", row.bets_open.is_some()),
        ("taker_fee_pct", row.taker_fee_pct.is_some()),
        ("maker_fee_pct", row.maker_fee_pct.is_some()),
        ("fee_exponent", row.fee_exponent.is_some()),
        ("reward_pct", row.reward_pct.is_some()),
    ];

    for (column, is_live) in live_columns {
        if is_live {
            mock.retain(|entry| entry != column);
        }
    }

    mock
}

#[cfg(feature = "discovery-sdk")]
fn market_bets_open_label(market: &SdkMarket) -> Option<String> {
    if let Some(accepting) = market.accepting_orders {
        return Some(if accepting { "open" } else { "closed" }.to_string());
    }
    if let Some(closed) = market.closed {
        return Some(if closed { "closed" } else { "open" }.to_string());
    }
    if let Some(active) = market.active {
        return Some(if active { "open" } else { "closed" }.to_string());
    }
    None
}

#[cfg(feature = "discovery-sdk")]
struct FeeParams {
    taker_fee_pct: String,
    maker_fee_pct: String,
    fee_exponent: String,
}

#[cfg(feature = "discovery-sdk")]
fn market_fee_params(market: &SdkMarket, duration: Duration) -> FeeParams {
    let fee_type = market
        .format_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            market
                .market_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });
    fee_params_from_type(fee_type, market.fees_enabled, duration)
}

#[cfg(feature = "discovery-sdk")]
fn fee_params_from_type(
    fee_type: Option<&str>,
    fees_enabled: Option<bool>,
    duration: Duration,
) -> FeeParams {
    let fees_enabled = fees_enabled.unwrap_or(false);
    if !fees_enabled {
        return FeeParams {
            taker_fee_pct: "0".to_string(),
            maker_fee_pct: "0".to_string(),
            fee_exponent: "-".to_string(),
        };
    }

    // Per dashboard spec: crypto_15_min fees profile when fees are enabled.
    if fees_enabled && matches!(fee_type, Some("crypto_15_min")) {
        return FeeParams {
            taker_fee_pct: "0.25".to_string(),
            maker_fee_pct: "-0.05".to_string(),
            fee_exponent: "2".to_string(),
        };
    }

    // SDK fallback: Gamma feeType is not exposed in current SDK Market type for these crypto rows.
    if fee_type.is_none() && matches!(duration, Duration::M5 | Duration::M15) {
        return FeeParams {
            taker_fee_pct: "0.25".to_string(),
            maker_fee_pct: "-0.05".to_string(),
            fee_exponent: "2".to_string(),
        };
    }

    if fee_type.is_none() {
        return FeeParams {
            taker_fee_pct: "0".to_string(),
            maker_fee_pct: "0".to_string(),
            fee_exponent: "-".to_string(),
        };
    }

    FeeParams {
        taker_fee_pct: "0".to_string(),
        maker_fee_pct: "0".to_string(),
        fee_exponent: "-".to_string(),
    }
}

#[cfg(feature = "discovery-sdk")]
fn market_reward_pct(market: &SdkMarket) -> Option<String> {
    if let Some(rewards) = market
        .clob_rewards
        .as_ref()
        .and_then(|rewards| rewards.first())
    {
        if let Some(rate) = &rewards.rewards_daily_rate {
            return Some(rate.to_string());
        }
        if let Some(amount) = &rewards.rewards_amount {
            return Some(amount.to_string());
        }
    }
    market.uma_reward.as_ref().map(|value| value.to_string())
}

pub fn demo_snapshot() -> DashboardSnapshot {
    let now_ts = Utc::now().timestamp();
    let offset = std::env::var("PMFLIPS_DISCOVERY_OFFSET_4H_MIN")
        .ok()
        .and_then(|raw| raw.parse::<i32>().ok())
        .unwrap_or(60);

    let slug_cfg = SlugConfig {
        discovery_offset_4h_min: offset,
    };

    let rows =
        build_previous_active_and_next_discovery_keys(now_ts, &ALL_COINS, &ALL_DURATIONS, slug_cfg)
            .unwrap_or_default()
            .into_iter()
            .map(scheduled_key_to_demo_row)
            .collect();

    DashboardSnapshot { rows }
}

fn scheduled_key_to_demo_row(scheduled: crate::discovery::ScheduledDiscoveryKey) -> DashboardRow {
    let start_ts_utc = scheduled.key.start_ts_utc;
    let end_ts_utc = interval_end_ts_utc(start_ts_utc, scheduled.key.duration);
    let mut row = DashboardRow::unresolved_with_times(
        scheduled.key.slug,
        coin_label(scheduled.key.coin),
        duration_label(scheduled.key.duration),
        start_ts_utc,
        end_ts_utc,
    );

    row.in_interval = Some(
        match scheduled.window {
            DiscoveryWindow::Active => "yes",
            DiscoveryWindow::Previous | DiscoveryWindow::Next => "no",
        }
        .to_string(),
    );

    row.end_hhmm = Some(utc_hhmm(end_ts_utc));
    row
}

fn coin_label(coin: Coin) -> &'static str {
    match coin {
        Coin::Btc => "BTC",
        Coin::Eth => "ETH",
        Coin::Sol => "SOL",
        Coin::Xrp => "XRP",
    }
}

fn duration_label(duration: Duration) -> &'static str {
    match duration {
        Duration::M5 => "5m",
        Duration::M15 => "15m",
        Duration::H1 => "1h",
        Duration::H4 => "4h",
        Duration::D1 => "1d",
    }
}

fn interval_end_ts_utc(start_ts_utc: i64, duration: Duration) -> i64 {
    match duration {
        Duration::M5 => start_ts_utc.saturating_add(5 * 60),
        Duration::M15 => start_ts_utc.saturating_add(15 * 60),
        Duration::H1 => start_ts_utc.saturating_add(60 * 60),
        Duration::H4 => {
            let start_ny = Utc
                .timestamp_opt(start_ts_utc, 0)
                .single()
                .expect("valid start timestamp expected")
                .with_timezone(&New_York);
            let date = start_ny.date_naive();
            let block_hour = (start_ny.hour() / 4) * 4;

            let next_local = if block_hour == 20 {
                let next_date = date
                    .checked_add_days(Days::new(1))
                    .expect("next date should exist");
                ny_datetime(next_date, 0).expect("valid next ET 4h boundary")
            } else {
                ny_datetime(date, block_hour + 4).expect("valid next ET 4h boundary")
            };

            next_local.with_timezone(&Utc).timestamp()
        }
        Duration::D1 => {
            let start_ny = Utc
                .timestamp_opt(start_ts_utc, 0)
                .single()
                .expect("valid start timestamp expected")
                .with_timezone(&New_York);
            let next_date = start_ny
                .date_naive()
                .checked_add_days(Days::new(1))
                .expect("next date should exist");
            ny_datetime(next_date, 12)
                .expect("valid next ET noon boundary")
                .with_timezone(&Utc)
                .timestamp()
        }
    }
}

fn ny_datetime(date: chrono::NaiveDate, hour: u32) -> Option<chrono::DateTime<chrono_tz::Tz>> {
    New_York
        .with_ymd_and_hms(date.year(), date.month(), date.day(), hour, 0, 0)
        .single()
}

fn default_mock_columns() -> Vec<String> {
    DASHBOARD_COLUMN_KEYS
        .iter()
        .skip(3)
        .filter(|entry| **entry != "in_interval" && **entry != "end")
        .map(|entry| (*entry).to_string())
        .collect()
}

fn row_matches_bets_open(row: &DashboardRow, filters: &DashboardFilters) -> bool {
    match parse_bets_open_value(row.bets_open.as_deref()) {
        Some(value) => filters.bets_open_selected(value),
        None => filters.allows_unknown_bets_open(),
    }
}

fn row_matches_in_interval(
    row: &DashboardRow,
    filters: &DashboardFilters,
    now_ts_utc: i64,
) -> bool {
    let in_interval = compute_in_interval(now_ts_utc, row.start_ts_utc, row.end_ts_utc);
    if in_interval {
        filters.in_interval_selected(InIntervalFilter::Yes)
    } else {
        filters.in_interval_selected(InIntervalFilter::No)
    }
}

fn parse_set_or_all(input: &[String], allowed: &[&str]) -> HashSet<String> {
    if input.is_empty() {
        return allowed.iter().map(|entry| (*entry).to_string()).collect();
    }

    let allowed_by_norm: std::collections::HashMap<String, &str> = allowed
        .iter()
        .map(|entry| (entry.trim().to_ascii_lowercase(), *entry))
        .collect();

    input
        .iter()
        .filter_map(|entry| {
            let normalized = entry.trim().to_ascii_lowercase();
            allowed_by_norm
                .get(&normalized)
                .map(|canonical| (*canonical).to_string())
        })
        .collect()
}

fn parse_bets_open(input: &[String]) -> HashSet<BetsOpenFilter> {
    if input.is_empty() {
        return HashSet::from([BetsOpenFilter::Open, BetsOpenFilter::Closed]);
    }

    input
        .iter()
        .filter_map(|entry| parse_bets_open_value(Some(entry)))
        .collect()
}

fn parse_bets_open_value(value: Option<&str>) -> Option<BetsOpenFilter> {
    match value.map(|entry| entry.trim().to_ascii_lowercase()) {
        Some(v) if v == "open" => Some(BetsOpenFilter::Open),
        Some(v) if v == "closed" => Some(BetsOpenFilter::Closed),
        _ => None,
    }
}

fn parse_in_interval(input: &[String]) -> HashSet<InIntervalFilter> {
    if input.is_empty() {
        return HashSet::from([InIntervalFilter::Yes, InIntervalFilter::No]);
    }

    input
        .iter()
        .filter_map(|entry| match entry.trim().to_ascii_lowercase().as_str() {
            "yes" => Some(InIntervalFilter::Yes),
            "no" => Some(InIntervalFilter::No),
            _ => None,
        })
        .collect()
}

fn utc_hhmm(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_column_value(column_key: &str, raw: Option<&str>) -> String {
    match column_key {
        "taker_fee_pct" | "maker_fee_pct" | "reward_pct" => {
            if raw.map(|entry| entry.trim().is_empty()).unwrap_or(true) {
                "0".to_string()
            } else {
                format_maybe_composite(raw.unwrap_or_default(), 3)
            }
        }
        "fee_exponent" => raw
            .map(|value| {
                if value.trim().is_empty() {
                    "-".to_string()
                } else {
                    value.to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string()),
        "probability" => raw
            .map(format_probability)
            .unwrap_or_else(|| "-".to_string()),
        "ref_price" | "price" | "best_bid_yes" | "best_ask_yes" | "position_net" | "pos_yes"
        | "pos_no" | "offer_yes" | "offer_no" | "net_profit" => raw
            .map(|value| format_maybe_composite(value, 4))
            .unwrap_or_else(|| "-".to_string()),
        _ => raw
            .map(|value| {
                if value.trim().is_empty() {
                    "-".to_string()
                } else {
                    value.to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string()),
    }
}

fn format_probability(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return "-".to_string();
    }

    let is_percent_input = trimmed.ends_with('%');
    let number_text = if is_percent_input {
        trimmed.trim_end_matches('%').trim()
    } else {
        trimmed
    };

    match number_text.parse::<f64>() {
        Ok(mut numeric) => {
            if !is_percent_input && numeric <= 1.0 {
                numeric *= 100.0;
            }
            format!("{}%", format_significant(numeric, 3))
        }
        Err(_) => trimmed.to_string(),
    }
}

fn format_maybe_composite(value: &str, sig_digits: usize) -> String {
    if value.contains('@') {
        return value
            .split('@')
            .map(|segment| format_numeric_or_keep(segment.trim(), sig_digits))
            .collect::<Vec<_>>()
            .join("@");
    }

    format_numeric_or_keep(value.trim(), sig_digits)
}

fn format_numeric_or_keep(value: &str, sig_digits: usize) -> String {
    match value.parse::<f64>() {
        Ok(number) => format_significant(number, sig_digits),
        Err(_) => value.to_string(),
    }
}

fn format_significant(value: f64, sig_digits: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }

    let abs = value.abs();
    let exp = abs.log10().floor() as i32;
    let decimals = sig_digits as i32 - exp - 1;

    let raw = if decimals >= 0 {
        format!("{:.*}", decimals as usize, value)
    } else {
        let factor = 10f64.powi(-decimals);
        let rounded = (value / factor).round() * factor;
        format!("{rounded:.0}")
    };

    trim_number(raw)
}

fn trim_number(raw: String) -> String {
    if let Some(dot_idx) = raw.find('.') {
        let mut trimmed = raw;
        while trimmed.ends_with('0') {
            trimmed.pop();
        }
        if trimmed.len() == dot_idx + 1 {
            trimmed.pop();
        }
        if trimmed == "-0" {
            "0".to_string()
        } else {
            trimmed
        }
    } else if raw == "-0" {
        "0".to_string()
    } else {
        raw
    }
}

fn render_checkbox_group<F>(
    title: &str,
    param_name: &str,
    options: &[&str],
    mut is_checked: F,
) -> String
where
    F: FnMut(&str) -> bool,
{
    let mut out = String::new();
    out.push_str("<section class=\"filter-block\">");
    out.push_str("<p class=\"filter-title\">");
    out.push_str(&escape_html(title));
    out.push_str("</p>");

    for option in options {
        let checked = if is_checked(option) { " checked" } else { "" };
        out.push_str("<label class=\"filter-item\"><input type=\"checkbox\" name=\"");
        out.push_str(&escape_html(param_name));
        out.push_str("\" value=\"");
        out.push_str(&escape_html(option));
        out.push_str("\"");
        out.push_str(checked);
        out.push_str("> <span>");
        out.push_str(&escape_html(option));
        out.push_str("</span></label>");
    }

    out.push_str("</section>");
    out
}

fn render_rows_html(rows: &[DashboardDisplayRow]) -> String {
    let mut out = String::new();
    for (idx, row) in rows.iter().enumerate() {
        out.push_str(&render_row_html(row, idx));
    }
    out
}

fn render_row_html(row: &DashboardDisplayRow, idx: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!("<tr data-row=\"{idx}\">"));

    let link_class = if row.mock_columns.iter().any(|entry| entry == "link") {
        "cell-mock"
    } else {
        ""
    };
    out.push_str(&format!("<td class=\"market-cell {}\">", link_class));
    out.push_str("<a class=\"market-btn\" target=\"_blank\" rel=\"noopener noreferrer\" href=\"");
    out.push_str(&escape_html(&row.link_url));
    out.push_str("\">Open Market</a>");
    out.push_str("<span class=\"slug-id\" title=\"");
    out.push_str(&escape_html(&row.slug));
    out.push_str("\">");
    out.push_str(&escape_html(&row.slug));
    out.push_str("</span></td>");

    let columns: [(&str, &str); 20] = [
        ("coin", &row.coin),
        ("duration", &row.duration),
        ("bets_open", &row.bets_open),
        ("in_interval", &row.in_interval),
        ("end", &row.end_hhmm),
        ("ref_price", &row.ref_price),
        ("price", &row.price),
        ("probability", &row.probability),
        ("best_bid_yes", &row.best_bid_yes),
        ("best_ask_yes", &row.best_ask_yes),
        ("position_net", &row.position_net),
        ("pos_yes", &row.pos_yes),
        ("pos_no", &row.pos_no),
        ("offer_yes", &row.offer_yes),
        ("offer_no", &row.offer_no),
        ("net_profit", &row.net_profit),
        ("taker_fee_pct", &row.taker_fee_pct),
        ("maker_fee_pct", &row.maker_fee_pct),
        ("fee_exponent", &row.fee_exponent),
        ("reward_pct", &row.reward_pct),
    ];

    for (key, value) in columns {
        let class = if row.mock_columns.iter().any(|entry| entry == key) {
            "cell-mock"
        } else {
            ""
        };

        if key == "end" {
            out.push_str("<td data-end-ts=\"");
            out.push_str(&row.end_ts_utc.to_string());
            out.push_str("\" class=\"");
            out.push_str(class);
            out.push_str("\">");
            out.push_str(&escape_html(value));
            out.push_str("</td>");
        } else {
            out.push_str("<td class=\"");
            out.push_str(class);
            out.push_str("\">");
            out.push_str(&escape_html(value));
            out.push_str("</td>");
        }
    }

    out.push_str("</tr>\n");
    out
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Clone)]
struct DashboardAppState {
    source: Arc<dyn DashboardSnapshotSource>,
}

async fn get_dashboard_html(
    State(state): State<DashboardAppState>,
    Query(query_pairs): Query<Vec<(String, String)>>,
) -> impl IntoResponse {
    let snapshot = state.source.snapshot();
    let query = dashboard_query_from_pairs(&query_pairs);
    let filters = DashboardFilters::from_query(&query);
    let html = render_dashboard_html_with_filters(&snapshot, &filters, Utc::now().timestamp());
    Html(html)
}

async fn get_dashboard_snapshot(
    State(state): State<DashboardAppState>,
    Query(query_pairs): Query<Vec<(String, String)>>,
) -> impl IntoResponse {
    let snapshot = state.source.snapshot();
    let query = dashboard_query_from_pairs(&query_pairs);
    let filters = DashboardFilters::from_query(&query);
    let display = build_display_snapshot(&snapshot, &filters, Utc::now().timestamp());
    Json(display)
}

fn dashboard_query_from_pairs(query_pairs: &[(String, String)]) -> DashboardQuery {
    let mut query = DashboardQuery::default();

    for (key, value) in query_pairs {
        match key.trim().to_ascii_lowercase().as_str() {
            "coin" => query.coin.push(value.clone()),
            "duration" => query.duration.push(value.clone()),
            "bets_open" => query.bets_open.push(value.clone()),
            "in_interval" => query.in_interval.push(value.clone()),
            _ => {}
        }
    }

    query
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(
        coin: &str,
        duration: &str,
        start: i64,
        end: i64,
        bets_open: Option<&str>,
    ) -> DashboardRow {
        DashboardRow {
            slug: format!("{}-{}-{}", coin.to_lowercase(), duration, start),
            coin: coin.to_string(),
            duration: duration.to_string(),
            start_ts_utc: start,
            end_ts_utc: end,
            bets_open: bets_open.map(|v| v.to_string()),
            in_interval: None,
            end_hhmm: None,
            ref_price: Some("0.4987654".to_string()),
            price: Some("0.5123456".to_string()),
            probability: Some("0.5123".to_string()),
            best_bid_yes: Some("0.51".to_string()),
            best_ask_yes: Some("0.5139".to_string()),
            position_net: Some("12.34567@0.498765@YES".to_string()),
            pos_yes: Some("1.23456@0.5".to_string()),
            pos_no: None,
            offer_yes: Some("2.34567@0.52".to_string()),
            offer_no: Some("3.33333@0.48".to_string()),
            net_profit: Some("0.123456".to_string()),
            taker_fee_pct: Some("0.25".to_string()),
            maker_fee_pct: Some("-0.05".to_string()),
            fee_exponent: Some("2".to_string()),
            reward_pct: Some("0.004567".to_string()),
            mock_columns: vec!["price".to_string()],
        }
    }

    #[test]
    fn header_order_and_column_count_are_exact() {
        assert_eq!(DASHBOARD_HEADERS.len(), 21);
        assert_eq!(DASHBOARD_COLUMN_KEYS.len(), 21);
        assert_eq!(DASHBOARD_HEADERS[0], "Link");
        assert_eq!(DASHBOARD_HEADERS[8], "Probability");
        assert_eq!(DASHBOARD_HEADERS[20], "Reward %");
    }

    #[cfg(feature = "discovery-sdk")]
    #[test]
    fn fee_params_match_crypto_15_min_profile() {
        let params = fee_params_from_type(Some("crypto_15_min"), Some(true), Duration::H1);
        assert_eq!(params.taker_fee_pct, "0.25");
        assert_eq!(params.maker_fee_pct, "-0.05");
        assert_eq!(params.fee_exponent, "2");
    }

    #[cfg(feature = "discovery-sdk")]
    #[test]
    fn fee_params_default_when_fee_type_missing_or_disabled() {
        let disabled = fee_params_from_type(Some("crypto_15_min"), Some(false), Duration::M15);
        assert_eq!(disabled.taker_fee_pct, "0");
        assert_eq!(disabled.maker_fee_pct, "0");
        assert_eq!(disabled.fee_exponent, "-");
    }

    #[cfg(feature = "discovery-sdk")]
    #[test]
    fn fee_params_fallback_for_5m_and_15m_when_type_missing() {
        let m5 = fee_params_from_type(None, Some(true), Duration::M5);
        assert_eq!(m5.taker_fee_pct, "0.25");
        assert_eq!(m5.maker_fee_pct, "-0.05");
        assert_eq!(m5.fee_exponent, "2");

        let m15 = fee_params_from_type(None, Some(true), Duration::M15);
        assert_eq!(m15.taker_fee_pct, "0.25");
        assert_eq!(m15.maker_fee_pct, "-0.05");
        assert_eq!(m15.fee_exponent, "2");

        let h1 = fee_params_from_type(None, Some(true), Duration::H1);
        assert_eq!(h1.taker_fee_pct, "0");
        assert_eq!(h1.maker_fee_pct, "0");
        assert_eq!(h1.fee_exponent, "-");

        let d1 = fee_params_from_type(None, Some(true), Duration::D1);
        assert_eq!(d1.taker_fee_pct, "0");
        assert_eq!(d1.maker_fee_pct, "0");
        assert_eq!(d1.fee_exponent, "-");
    }

    #[cfg(feature = "discovery-sdk")]
    #[test]
    fn fee_params_no_type_and_fees_disabled_is_zero() {
        let params = fee_params_from_type(None, Some(false), Duration::M5);
        assert_eq!(params.taker_fee_pct, "0");
        assert_eq!(params.maker_fee_pct, "0");
        assert_eq!(params.fee_exponent, "-");
    }

    #[test]
    fn filter_defaults_select_all() {
        let filters = DashboardFilters::from_query(&DashboardQuery::default());
        assert_eq!(filters.coins.len(), 4);
        assert_eq!(filters.durations.len(), 5);
        assert!(filters.bets_open_selected(BetsOpenFilter::Open));
        assert!(filters.bets_open_selected(BetsOpenFilter::Closed));
        assert!(filters.in_interval_selected(InIntervalFilter::Yes));
        assert!(filters.in_interval_selected(InIntervalFilter::No));
    }

    #[test]
    fn filter_logic_or_within_and_across() {
        let query = DashboardQuery {
            coin: vec!["BTC".to_string(), "ETH".to_string()],
            duration: vec!["1h".to_string()],
            bets_open: vec!["open".to_string()],
            in_interval: vec!["yes".to_string()],
        };
        let filters = DashboardFilters::from_query(&query);
        let now = 1_000;

        let rows = vec![
            sample_row("BTC", "1h", 900, 1100, Some("open")),
            sample_row("ETH", "1h", 900, 1100, Some("open")),
            sample_row("SOL", "1h", 900, 1100, Some("open")),
            sample_row("BTC", "5m", 900, 1100, Some("open")),
            sample_row("BTC", "1h", 900, 1100, Some("closed")),
        ];

        let filtered = apply_filters(&rows, &filters, now);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].coin, "BTC");
        assert_eq!(filtered[1].coin, "ETH");
    }

    #[test]
    fn in_interval_boundary_is_start_inclusive_end_exclusive() {
        assert!(compute_in_interval(100, 100, 200));
        assert!(compute_in_interval(199, 100, 200));
        assert!(!compute_in_interval(200, 100, 200));
    }

    #[test]
    fn formatting_rules_are_applied() {
        let row = sample_row("BTC", "1h", 100, 200, Some("open"));
        let display = format_row_for_display(&row, 150);

        assert_eq!(display.ref_price, "0.4988");
        assert_eq!(display.price, "0.5123");
        assert_eq!(display.position_net, "12.35@0.4988@YES");
        assert_eq!(display.taker_fee_pct, "0.25");
        assert_eq!(display.maker_fee_pct, "-0.05");
        assert_eq!(display.fee_exponent, "2");
        assert_eq!(display.reward_pct, "0.00457");
        assert_eq!(display.probability, "51.2%");
    }

    #[test]
    fn unresolved_row_remains_visible_with_placeholders_and_mock_columns() {
        let row = DashboardRow::unresolved_with_times("xrp-updown-15m-2", "XRP", "15m", 100, 200);
        let display = format_row_for_display(&row, 150);

        assert_eq!(display.slug, "xrp-updown-15m-2");
        assert_eq!(display.coin, "XRP");
        assert_eq!(display.duration, "15m");
        assert_eq!(display.bets_open, "-");
        assert_eq!(display.ref_price, "-");
        assert_eq!(display.price, "-");
        assert_eq!(display.probability, "-");
        assert_eq!(display.taker_fee_pct, "0");
        assert_eq!(display.maker_fee_pct, "0");
        assert_eq!(display.fee_exponent, "-");
        assert!(display.mock_columns.contains(&"price".to_string()));
    }

    #[test]
    fn link_generation_uses_slug_url() {
        let link = market_link("btc-updown-5m-123");
        assert_eq!(link, "https://polymarket.com/event/btc-updown-5m-123");
    }

    #[test]
    fn demo_snapshot_contains_60_rows() {
        let snapshot = demo_snapshot();
        assert_eq!(snapshot.rows.len(), 60);
    }

    #[test]
    fn rendered_html_has_button_mock_and_polling_script() {
        let snapshot = DashboardSnapshot {
            rows: vec![DashboardRow::unresolved_with_times(
                "eth-updown-15m-9",
                "ETH",
                "15m",
                100,
                200,
            )],
        };

        let html = render_dashboard_html(&snapshot);
        assert!(html.contains("market-btn"));
        assert!(html.contains("Open Market"));
        assert!(html.contains("cell-mock"));
        assert!(html.contains("setInterval(refresh, 250)"));
    }
}

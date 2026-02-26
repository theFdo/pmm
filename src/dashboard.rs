//! Step 3 dashboard core table primitives and HTTP routes.

use std::sync::{Arc, RwLock};

use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::discovery::{
    build_previous_active_and_next_discovery_keys, DiscoveryWindow, ALL_COINS, ALL_DURATIONS,
};
use crate::slug::{Coin, Duration, SlugConfig};

pub const DASHBOARD_HEADERS: [&str; 22] = [
    "Link",
    "Coin",
    "Duration",
    "Bets Open",
    "In Interval",
    "End",
    "Midprice",
    "Best Bid YES",
    "Best Ask YES",
    "Position Net",
    "Pos YES",
    "Pos NO",
    "Offer YES",
    "Offer NO",
    "Net Profit",
    "Fee %",
    "Reward %",
    "P_finished",
    "P_running",
    "P_next",
    "dist1(mu,sigma,nu,lambda)",
    "dist2(mu,sigma,nu,lambda)",
];

pub const DASHBOARD_COLUMN_KEYS: [&str; 22] = [
    "link",
    "coin",
    "duration",
    "bets_open",
    "in_interval",
    "end",
    "midprice",
    "best_bid_yes",
    "best_ask_yes",
    "position_net",
    "pos_yes",
    "pos_no",
    "offer_yes",
    "offer_no",
    "net_profit",
    "fee_pct",
    "reward_pct",
    "p_finished",
    "p_running",
    "p_next",
    "dist1",
    "dist2",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub rows: Vec<DashboardRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardRow {
    pub slug: String,
    pub coin: String,
    pub duration: String,
    pub bets_open: Option<String>,
    pub in_interval: Option<String>,
    pub end_hhmm: Option<String>,
    pub midprice: Option<String>,
    pub best_bid_yes: Option<String>,
    pub best_ask_yes: Option<String>,
    pub position_net: Option<String>,
    pub pos_yes: Option<String>,
    pub pos_no: Option<String>,
    pub offer_yes: Option<String>,
    pub offer_no: Option<String>,
    pub net_profit: Option<String>,
    pub fee_pct: Option<String>,
    pub reward_pct: Option<String>,
    pub p_finished: Option<String>,
    pub p_running: Option<String>,
    pub p_next: Option<String>,
    pub dist1: Option<String>,
    pub dist2: Option<String>,
    pub mock_columns: Vec<String>,
}

impl DashboardRow {
    pub fn unresolved(
        slug: impl Into<String>,
        coin: impl Into<String>,
        duration: impl Into<String>,
    ) -> Self {
        Self {
            slug: slug.into(),
            coin: coin.into(),
            duration: duration.into(),
            bets_open: None,
            in_interval: None,
            end_hhmm: None,
            midprice: None,
            best_bid_yes: None,
            best_ask_yes: None,
            position_net: None,
            pos_yes: None,
            pos_no: None,
            offer_yes: None,
            offer_no: None,
            net_profit: None,
            fee_pct: None,
            reward_pct: None,
            p_finished: None,
            p_running: None,
            p_next: None,
            dist1: None,
            dist2: None,
            mock_columns: default_mock_columns(),
        }
    }

    pub fn to_cell_text_values(&self) -> Vec<String> {
        vec![
            self.slug.clone(),
            self.coin.clone(),
            self.duration.clone(),
            display_or_dash(&self.bets_open),
            display_or_dash(&self.in_interval),
            display_or_dash(&self.end_hhmm),
            display_or_dash(&self.midprice),
            display_or_dash(&self.best_bid_yes),
            display_or_dash(&self.best_ask_yes),
            display_or_dash(&self.position_net),
            display_or_dash(&self.pos_yes),
            display_or_dash(&self.pos_no),
            display_or_dash(&self.offer_yes),
            display_or_dash(&self.offer_no),
            display_or_dash(&self.net_profit),
            display_or_dash(&self.fee_pct),
            display_or_dash(&self.reward_pct),
            display_or_dash(&self.p_finished),
            display_or_dash(&self.p_running),
            display_or_dash(&self.p_next),
            display_or_dash(&self.dist1),
            display_or_dash(&self.dist2),
        ]
    }

    pub fn is_mock_column(&self, column_key: &str) -> bool {
        self.mock_columns.iter().any(|entry| entry == column_key)
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

pub fn dashboard_router(source: Arc<dyn DashboardSnapshotSource>) -> Router {
    Router::new()
        .route("/dashboard", get(get_dashboard_html))
        .route("/dashboard/snapshot", get(get_dashboard_snapshot))
        .with_state(DashboardAppState { source })
}

pub fn market_link(slug: &str) -> String {
    format!("https://polymarket.com/event/{slug}")
}

pub fn render_dashboard_html(snapshot: &DashboardSnapshot) -> String {
    let now_utc = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let mut out = String::new();
    out.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>PMM Dashboard</title>\n");
    out.push_str("<style>:root{--bg:#f5f1e7;--bg2:#e9f0f2;--card:#ffffff;--ink:#182026;--muted:#5f6a73;--line:#d7dce1;--head:#14343f;--btn:#0c5f78;--btnhover:#094d61;--mockbg:#fff5b8;--mockink:#555c63}*{box-sizing:border-box}body{margin:0;color:var(--ink);font-family:\"Space Grotesk\",\"Avenir Next\",\"Segoe UI\",sans-serif;background:radial-gradient(circle at 10% 5%, #ffe7a3 0%, transparent 30%),radial-gradient(circle at 90% 0%, #b9e5f0 0%, transparent 28%),linear-gradient(160deg,var(--bg),var(--bg2));min-height:100vh}.shell{max-width:1500px;margin:0 auto;padding:24px 18px 28px}.hero{background:linear-gradient(135deg,#102f3a 0%,#24576b 100%);color:#f7fbfc;border-radius:16px;padding:18px 20px;box-shadow:0 10px 30px rgba(16,47,58,.25)}.hero h1{margin:0 0 8px;font-size:1.6rem;letter-spacing:.01em}.hero-meta{display:flex;gap:16px;flex-wrap:wrap;font-size:.92rem;color:#dcebf0}.card{margin-top:16px;background:var(--card);border:1px solid #cbd4db;border-radius:16px;overflow:hidden;box-shadow:0 12px 28px rgba(26,35,42,.12)}.table-wrap{overflow:auto;max-height:75vh}table{width:100%;border-collapse:collapse;min-width:1300px}thead th{position:sticky;top:0;z-index:2;background:var(--head);color:#f2f7f9;font-size:.8rem;text-transform:uppercase;letter-spacing:.04em;padding:10px 10px;border-bottom:1px solid #0e2730}tbody td{font-size:.84rem;padding:9px 10px;border-bottom:1px solid var(--line);white-space:nowrap}tbody tr:nth-child(even){background:#fafcfd}.market-cell{min-width:210px}.market-btn{display:inline-flex;align-items:center;justify-content:center;background:linear-gradient(135deg,var(--btn),#0f7592);color:#fff;text-decoration:none;padding:7px 10px;border-radius:9px;font-weight:700;font-size:.76rem;border:1px solid rgba(0,0,0,.12);box-shadow:0 2px 8px rgba(12,95,120,.25)}.market-btn:hover{background:linear-gradient(135deg,var(--btnhover),#0d5f78)}.slug-id{display:block;margin-top:6px;font-family:\"IBM Plex Mono\",\"SFMono-Regular\",monospace;font-size:.67rem;color:var(--muted);max-width:240px;overflow:hidden;text-overflow:ellipsis}.cell-mock{background:linear-gradient(135deg,var(--mockbg) 0%, #fff3ca 100%);color:var(--mockink)}.cell-mock::after{content:\" M\";font-size:.62rem;font-weight:700;color:#8c6a00}.legend{padding:10px 14px;border-top:1px solid var(--line);font-size:.8rem;color:var(--muted);background:#f8fbfc}.legend b{color:#8c6a00}@media (max-width:760px){.hero h1{font-size:1.28rem}.shell{padding:12px}.card{margin-top:12px;border-radius:12px}}</style>\n");
    out.push_str("</head><body><main class=\"shell\">\n");
    out.push_str("<section class=\"hero\"><h1>PMM Dashboard</h1>");
    out.push_str("<div class=\"hero-meta\">\n");
    out.push_str("<span>Scope: 4 coins × 4 durations × previous/active/next</span>");
    out.push_str(&format!("<span>Rows: {}</span>", snapshot.rows.len()));
    out.push_str(&format!(
        "<span>Generated: {}</span>",
        escape_html(&now_utc)
    ));
    out.push_str("</div></section>\n");
    out.push_str(
        "<section class=\"card\"><div class=\"table-wrap\"><table id=\"dashboard-table\">\n",
    );
    out.push_str("<thead><tr>");
    for header in DASHBOARD_HEADERS {
        out.push_str("<th>");
        out.push_str(&escape_html(header));
        out.push_str("</th>");
    }
    out.push_str("</tr></thead><tbody>\n");

    for (idx, row) in snapshot.rows.iter().enumerate() {
        let values = row.to_cell_text_values();
        out.push_str(&format!("<tr data-row=\"{idx}\">"));

        let link_url = market_link(&row.slug);
        let link_class = if row.is_mock_column(DASHBOARD_COLUMN_KEYS[0]) {
            "cell-mock"
        } else {
            ""
        };
        out.push_str(&format!("<td class=\"market-cell {}\">", link_class));
        out.push_str(
            "<a class=\"market-btn\" target=\"_blank\" rel=\"noopener noreferrer\" href=\"",
        );
        out.push_str(&escape_html(&link_url));
        out.push_str("\">Open Market</a>");
        out.push_str("<span class=\"slug-id\" title=\"");
        out.push_str(&escape_html(&values[0]));
        out.push_str("\">");
        out.push_str(&escape_html(&values[0]));
        out.push_str("</span></td>");

        for (col_idx, value) in values.iter().enumerate().skip(1) {
            let key = DASHBOARD_COLUMN_KEYS[col_idx];
            let class = if row.is_mock_column(key) {
                "cell-mock"
            } else {
                ""
            };
            out.push_str(&format!("<td class=\"{}\">", class));
            out.push_str(&escape_html(value));
            out.push_str("</td>");
        }

        out.push_str("</tr>\n");
    }

    out.push_str("</tbody></table></div><div class=\"legend\">Mock-backed cells are highlighted <b>yellow/grey</b> and tagged with <b>M</b>.</div></section>");
    out.push_str("</main></body></html>\n");
    out
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
    let mut row = DashboardRow::unresolved(
        scheduled.key.slug,
        coin_label(scheduled.key.coin),
        duration_label(scheduled.key.duration),
    );

    row.in_interval = Some(
        match scheduled.window {
            DiscoveryWindow::Active => "yes",
            DiscoveryWindow::Previous | DiscoveryWindow::Next => "no",
        }
        .to_string(),
    );

    let end_ts = scheduled
        .key
        .start_ts_utc
        .saturating_add(duration_seconds(scheduled.key.duration));

    row.end_hhmm = Utc
        .timestamp_opt(end_ts, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string());

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
    }
}

fn duration_seconds(duration: Duration) -> i64 {
    match duration {
        Duration::M5 => 5 * 60,
        Duration::M15 => 15 * 60,
        Duration::H1 => 60 * 60,
        Duration::H4 => 4 * 60 * 60,
    }
}

fn default_mock_columns() -> Vec<String> {
    DASHBOARD_COLUMN_KEYS
        .iter()
        .skip(3)
        .map(|entry| (*entry).to_string())
        .collect()
}

fn display_or_dash(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "-".to_string())
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

async fn get_dashboard_html(State(state): State<DashboardAppState>) -> impl IntoResponse {
    let snapshot = state.source.snapshot();
    Html(render_dashboard_html(&snapshot))
}

async fn get_dashboard_snapshot(State(state): State<DashboardAppState>) -> impl IntoResponse {
    let snapshot = state.source.snapshot();
    Json(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_order_and_column_count_are_exact() {
        assert_eq!(DASHBOARD_HEADERS.len(), 22);
        assert_eq!(DASHBOARD_COLUMN_KEYS.len(), 22);
        assert_eq!(DASHBOARD_HEADERS[0], "Link");
        assert_eq!(DASHBOARD_HEADERS[21], "dist2(mu,sigma,nu,lambda)");
    }

    #[test]
    fn row_to_cell_mapping_fills_all_columns() {
        let row = DashboardRow {
            slug: "btc-updown-5m-1".to_string(),
            coin: "BTC".to_string(),
            duration: "5m".to_string(),
            bets_open: Some("open".to_string()),
            in_interval: Some("yes".to_string()),
            end_hhmm: Some("00:05".to_string()),
            midprice: Some("1".to_string()),
            best_bid_yes: Some("2".to_string()),
            best_ask_yes: Some("3".to_string()),
            position_net: Some("4".to_string()),
            pos_yes: Some("5".to_string()),
            pos_no: Some("6".to_string()),
            offer_yes: Some("7".to_string()),
            offer_no: Some("8".to_string()),
            net_profit: Some("9".to_string()),
            fee_pct: Some("10".to_string()),
            reward_pct: Some("11".to_string()),
            p_finished: Some("12".to_string()),
            p_running: Some("13".to_string()),
            p_next: Some("14".to_string()),
            dist1: Some("15".to_string()),
            dist2: Some("16".to_string()),
            mock_columns: vec!["midprice".to_string()],
        };

        let cells = row.to_cell_text_values();
        assert_eq!(cells.len(), 22);
        assert_eq!(cells[0], "btc-updown-5m-1");
        assert_eq!(cells[21], "16");
        assert!(row.is_mock_column("midprice"));
    }

    #[test]
    fn unresolved_row_remains_visible_with_placeholders_and_mock_columns() {
        let row = DashboardRow::unresolved("xrp-updown-15m-2", "XRP", "15m");
        let cells = row.to_cell_text_values();

        assert_eq!(cells.len(), 22);
        assert_eq!(cells[0], "xrp-updown-15m-2");
        assert_eq!(cells[1], "XRP");
        assert_eq!(cells[2], "15m");
        assert_eq!(cells[3], "-");
        assert_eq!(cells[21], "-");
        assert!(row.is_mock_column("midprice"));
    }

    #[test]
    fn link_generation_uses_slug_url() {
        let link = market_link("btc-updown-5m-123");
        assert_eq!(link, "https://polymarket.com/event/btc-updown-5m-123");
    }

    #[test]
    fn demo_snapshot_contains_48_rows() {
        let snapshot = demo_snapshot();
        assert_eq!(snapshot.rows.len(), 48);
    }

    #[test]
    fn rendered_html_has_button_and_mock_class() {
        let snapshot = DashboardSnapshot {
            rows: vec![DashboardRow::unresolved("eth-updown-15m-9", "ETH", "15m")],
        };

        let html = render_dashboard_html(&snapshot);
        assert!(html.contains("market-btn"));
        assert!(html.contains("Open Market"));
        assert!(html.contains("cell-mock"));
    }
}

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use pmm::{
    dashboard_router, demo_snapshot, DashboardRow, DashboardSnapshot, InMemoryMockSnapshotSource,
};
use tower::util::ServiceExt;

fn row(coin: &str, duration: &str, start: i64, end: i64, bets_open: Option<&str>) -> DashboardRow {
    DashboardRow {
        slug: format!("{}-{}-{}", coin.to_lowercase(), duration, start),
        coin: coin.to_string(),
        duration: duration.to_string(),
        start_ts_utc: start,
        end_ts_utc: end,
        bets_open: bets_open.map(|v| v.to_string()),
        in_interval: None,
        end_hhmm: None,
        midprice: Some("0.5123456".to_string()),
        best_bid_yes: Some("0.49".to_string()),
        best_ask_yes: Some("0.51".to_string()),
        position_net: Some("1.23456@0.5@YES".to_string()),
        pos_yes: Some("1.2@0.5".to_string()),
        pos_no: None,
        offer_yes: Some("1.9@0.51".to_string()),
        offer_no: Some("1.8@0.49".to_string()),
        net_profit: Some("0.001234".to_string()),
        fee_pct: None,
        reward_pct: Some("0.004567".to_string()),
        p_finished: None,
        p_running: Some("0.5".to_string()),
        p_next: Some("52".to_string()),
        dist1: Some("(0.123456,1.9999,8.1,0)".to_string()),
        dist2: Some("(0.23456,1.8888,8.2,0)".to_string()),
        mock_columns: vec!["midprice".to_string()],
    }
}

#[tokio::test]
async fn dashboard_page_returns_table_filters_and_polling_script() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![row("BTC", "5m", 100, 200, Some("open"))],
    }));

    let app = dashboard_router(source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("<table"));
    assert!(text.contains("name=\"coin\""));
    assert!(text.contains("name=\"duration\""));
    assert!(text.contains("name=\"bets_open\""));
    assert!(text.contains("name=\"in_interval\""));
    assert!(text.contains("filters-form"));
    assert!(text.contains("addEventListener('change'"));
    assert!(text.contains("Auto-applies on checkbox change"));
    assert!(text.contains("setInterval(refresh, 100)"));
    assert!(text.contains("market-btn"));
    assert!(!text.contains("btn-apply"));
}

#[tokio::test]
async fn snapshot_endpoint_applies_query_filters() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![
            row("BTC", "1h", 100, 300, Some("open")),
            row("ETH", "1h", 100, 300, Some("open")),
            row("BTC", "5m", 100, 300, Some("open")),
            row("BTC", "1h", 100, 300, Some("closed")),
        ],
    }));

    let app = dashboard_router(source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/snapshot?coin=BTC&duration=1h&bets_open=open")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = json["rows"].as_array().unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["coin"], "BTC");
    assert_eq!(rows[0]["duration"], "1h");
    assert_eq!(rows[0]["bets_open"], "open");
}

#[tokio::test]
async fn snapshot_endpoint_supports_repeated_coin_query_params() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![
            row("BTC", "1h", 100, 300, Some("open")),
            row("ETH", "1h", 100, 300, Some("open")),
            row("SOL", "1h", 100, 300, Some("open")),
        ],
    }));

    let app = dashboard_router(source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/snapshot?coin=BTC&coin=ETH&duration=1h&bets_open=open")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = json["rows"].as_array().unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["coin"], "BTC");
    assert_eq!(rows[1]["coin"], "ETH");
}

#[tokio::test]
async fn snapshot_endpoint_applies_in_interval_boundary_logic() {
    let now = chrono::Utc::now().timestamp();

    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![
            row("BTC", "5m", now - 60, now + 240, Some("open")),
            row("ETH", "5m", now - 300, now, Some("open")),
        ],
    }));

    let app = dashboard_router(source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/snapshot?in_interval=yes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = json["rows"].as_array().unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["coin"], "BTC");
    assert_eq!(rows[0]["in_interval"], "yes");
}

#[tokio::test]
async fn snapshot_endpoint_formats_values_and_preserves_unresolved_rows() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![
            row("BTC", "5m", 100, 200, Some("open")),
            DashboardRow::unresolved_with_times("xrp-5m-1", "XRP", "5m", 100, 200),
        ],
    }));

    let app = dashboard_router(source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/snapshot")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = json["rows"].as_array().unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["midprice"], "0.5123");
    assert_eq!(rows[0]["reward_pct"], "0.00457");
    assert_eq!(rows[0]["p_running"], "50%");
    assert_eq!(rows[1]["coin"], "XRP");
    assert_eq!(rows[1]["midprice"], "-");
}

#[tokio::test]
async fn demo_snapshot_route_exposes_48_rows() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(demo_snapshot()));

    let app = dashboard_router(source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/snapshot")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["rows"].as_array().unwrap().len(), 48);
}

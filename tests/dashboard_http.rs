use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use pmm::{
    dashboard_router, demo_snapshot, DashboardRow, DashboardSnapshot, InMemoryMockSnapshotSource,
};
use tower::util::ServiceExt;

#[tokio::test]
async fn dashboard_page_returns_table_and_required_headers() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![DashboardRow {
            slug: "btc-updown-5m-1".to_string(),
            coin: "BTC".to_string(),
            duration: "5m".to_string(),
            bets_open: Some("open".to_string()),
            in_interval: Some("yes".to_string()),
            end_hhmm: Some("00:05".to_string()),
            midprice: Some("0.5".to_string()),
            best_bid_yes: Some("0.49".to_string()),
            best_ask_yes: Some("0.51".to_string()),
            position_net: Some("1@0.5@YES".to_string()),
            pos_yes: Some("1@0.5".to_string()),
            pos_no: None,
            offer_yes: Some("1@0.51".to_string()),
            offer_no: Some("1@0.49".to_string()),
            net_profit: Some("0".to_string()),
            fee_pct: Some("0".to_string()),
            reward_pct: Some("0".to_string()),
            p_finished: Some("-".to_string()),
            p_running: Some("50%".to_string()),
            p_next: Some("50%".to_string()),
            dist1: Some("(0,1,8,0)".to_string()),
            dist2: Some("(0,1,8,0)".to_string()),
            mock_columns: vec!["midprice".to_string(), "best_bid_yes".to_string()],
        }],
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
    assert!(text.contains("<th>Link</th>"));
    assert!(text.contains("<th>Coin</th>"));
    assert!(text.contains("<th>Duration</th>"));
    assert!(text.contains("<th>dist2(mu,sigma,nu,lambda)</th>"));
    assert!(text.contains("market-btn"));
}

#[tokio::test]
async fn mixed_rows_render_without_dropping_unresolved() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![
            DashboardRow::unresolved("eth-updown-15m-10", "ETH", "15m"),
            DashboardRow::unresolved("btc-updown-5m-20", "BTC", "5m"),
        ],
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

    let rendered_rows = text.matches("<tr data-row=").count();
    assert_eq!(rendered_rows, 2);
    assert!(text.contains("eth-updown-15m-10"));
    assert!(text.contains("btc-updown-5m-20"));
    assert!(text.contains("cell-mock"));
}

#[tokio::test]
async fn snapshot_endpoint_returns_mock_rows() {
    let source = Arc::new(InMemoryMockSnapshotSource::new(DashboardSnapshot {
        rows: vec![DashboardRow::unresolved("xrp-updown-4h-30", "XRP", "4h")],
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

    assert_eq!(json["rows"].as_array().unwrap().len(), 1);
    assert_eq!(json["rows"][0]["slug"], "xrp-updown-4h-30");
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

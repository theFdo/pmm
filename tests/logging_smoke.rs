use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use pmm::{
    dashboard_router, demo_snapshot, log_app_bind, log_app_start, log_source_selected,
    resolve_discovery_batch_with_fetcher, Coin, DiscoveryConfig, DiscoveryError, DiscoveryKey,
    DiscoveryStatus, Duration as MarketDuration, InMemoryMockSnapshotSource, LoggingConfig,
    SlugFetchOutcome,
};
use tower::util::ServiceExt;
use tracing::dispatcher::with_default;
use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriter;

#[derive(Clone, Default)]
struct SharedWriter {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl SharedWriter {
    fn output_string(&self) -> String {
        let bytes = self
            .inner
            .lock()
            .expect("writer lock should not be poisoned");
        String::from_utf8_lossy(&bytes).to_string()
    }
}

struct SharedWriterGuard {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriterGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriterGuard {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Write for SharedWriterGuard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = self
            .inner
            .lock()
            .expect("writer lock should not be poisoned");
        out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_logs(max_level: Level, f: impl FnOnce()) -> String {
    let writer = SharedWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_max_level(max_level)
        .with_writer(writer.clone())
        .finish();
    let dispatch = tracing::Dispatch::new(subscriber);

    with_default(&dispatch, f);
    writer.output_string()
}

fn sample_key(slug: &str) -> DiscoveryKey {
    DiscoveryKey::from_slug(
        Coin::Btc,
        MarketDuration::M5,
        1_735_689_900,
        slug.to_string(),
    )
}

#[test]
fn discovery_logs_batch_transport_failures() {
    let cfg = DiscoveryConfig::default();
    let keys = vec![sample_key("btc-updown-5m-a"), sample_key("btc-updown-5m-b")];
    let logs = capture_logs(Level::INFO, || {
        let err = resolve_discovery_batch_with_fetcher::<String, _>(&keys, &cfg, |_slugs| {
            Err(DiscoveryError::Transport(
                "simulated gamma outage".to_string(),
            ))
        })
        .expect_err("fetcher error should bubble up");

        assert!(matches!(err, DiscoveryError::Transport(_)));
    });

    assert!(logs.contains("\"event\":\"discovery.resolve.error\""));
    assert!(logs.contains("\"event\":\"discovery.degraded.batch_transport\""));
}

#[test]
fn discovery_logs_row_transport_degraded_events_at_debug() {
    let cfg = DiscoveryConfig::default();
    let keys = vec![sample_key("btc-updown-5m-row")];

    let logs = capture_logs(Level::DEBUG, || {
        let rows = resolve_discovery_batch_with_fetcher::<String, _>(&keys, &cfg, |_slugs| {
            let mut out = HashMap::new();
            out.insert(
                "btc-updown-5m-row".to_string(),
                SlugFetchOutcome::TransportError("timeout".to_string()),
            );
            Ok(out)
        })
        .expect("row-level transport errors should be materialized as unresolved rows");

        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].status, DiscoveryStatus::Unresolved { .. }));
    });

    assert!(logs.contains("\"event\":\"discovery.degraded.row_transport\""));
}

#[test]
fn server_lifecycle_helpers_emit_baseline_events() {
    let logs = capture_logs(Level::INFO, || {
        let cfg = LoggingConfig::default();
        log_app_start(&cfg);
        log_source_selected("demo", Some("PMM_DASHBOARD_USE_DEMO"), None);
        log_app_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080));
    });

    assert!(logs.contains("\"event\":\"app.start\""));
    assert!(logs.contains("\"event\":\"source.selected\""));
    assert!(logs.contains("\"event\":\"app.bind\""));
}

#[test]
fn snapshot_route_emits_http_snapshot_event() {
    let logs = capture_logs(Level::INFO, || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("single-thread runtime should build");

        rt.block_on(async {
            let source = Arc::new(InMemoryMockSnapshotSource::new(demo_snapshot()));
            let app = dashboard_router(source);

            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/dashboard/snapshot")
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("snapshot request should succeed");

            assert_eq!(response.status(), StatusCode::OK);
        });
    });

    assert!(logs.contains("\"event\":\"http.snapshot.request\""));
}

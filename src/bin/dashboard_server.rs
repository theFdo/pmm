use std::{net::SocketAddr, sync::Arc};

use pmm::{
    dashboard_router, init_logging, log_app_bind, log_app_start, log_source_selected,
    logging_config_from_env, DashboardSnapshotSource, InMemoryMockSnapshotSource,
};
#[cfg(feature = "discovery-sdk")]
use pmm::{LiveDiscoveryConfig, LiveDiscoverySnapshotSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let logging_cfg = logging_config_from_env();
    init_logging(&logging_cfg)?;
    log_app_start(&logging_cfg);

    let addr: SocketAddr = std::env::var("PMM_DASHBOARD_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()?;

    let source: Arc<dyn DashboardSnapshotSource> = source_from_env();
    let app = dashboard_router(source);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;

    log_app_bind(bound_addr);
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(feature = "discovery-sdk")]
fn source_from_env() -> Arc<dyn DashboardSnapshotSource> {
    let force_demo = std::env::var("PMM_DASHBOARD_USE_DEMO")
        .map(|raw| raw == "1" || raw.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if force_demo {
        log_source_selected("demo", Some("PMM_DASHBOARD_USE_DEMO"), None);
        Arc::new(InMemoryMockSnapshotSource::demo())
    } else {
        let cfg = LiveDiscoveryConfig::default();
        log_source_selected("live_discovery", None, Some(cfg.refresh_interval_ms));
        Arc::new(LiveDiscoverySnapshotSource::spawn(cfg))
    }
}

#[cfg(not(feature = "discovery-sdk"))]
fn source_from_env() -> Arc<dyn DashboardSnapshotSource> {
    log_source_selected("demo", Some("discovery_sdk_disabled"), None);
    Arc::new(InMemoryMockSnapshotSource::demo())
}

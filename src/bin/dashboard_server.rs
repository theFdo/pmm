use std::{net::SocketAddr, sync::Arc};

use pmm::{dashboard_router, DashboardSnapshotSource, InMemoryMockSnapshotSource};
#[cfg(feature = "discovery-sdk")]
use pmm::{LiveDiscoveryConfig, LiveDiscoverySnapshotSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("PMM_DASHBOARD_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()?;

    let source: Arc<dyn DashboardSnapshotSource> = source_from_env();
    let app = dashboard_router(source);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("dashboard listening on http://{addr}/dashboard");
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(feature = "discovery-sdk")]
fn source_from_env() -> Arc<dyn DashboardSnapshotSource> {
    let force_demo = std::env::var("PMM_DASHBOARD_USE_DEMO")
        .map(|raw| raw == "1" || raw.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if force_demo {
        eprintln!("dashboard: using demo snapshot source (PMM_DASHBOARD_USE_DEMO)");
        Arc::new(InMemoryMockSnapshotSource::demo())
    } else {
        let cfg = LiveDiscoveryConfig::default();
        eprintln!(
            "dashboard: using live discovery source (refresh {}ms)",
            cfg.refresh_interval_ms
        );
        Arc::new(LiveDiscoverySnapshotSource::spawn(cfg))
    }
}

#[cfg(not(feature = "discovery-sdk"))]
fn source_from_env() -> Arc<dyn DashboardSnapshotSource> {
    eprintln!("dashboard: discovery-sdk disabled; using demo snapshot source");
    Arc::new(InMemoryMockSnapshotSource::demo())
}

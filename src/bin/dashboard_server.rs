use std::{net::SocketAddr, sync::Arc};

use pmm::{dashboard_router, InMemoryMockSnapshotSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("PMM_DASHBOARD_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()?;

    let source = Arc::new(InMemoryMockSnapshotSource::demo());
    let app = dashboard_router(source);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("dashboard listening on http://{addr}/dashboard");
    axum::serve(listener, app).await?;

    Ok(())
}

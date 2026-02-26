//! PMM core crate.
//!
//! Current implemented scope:
//! - Step 1: deterministic slug generation
//! - Step 2: discovery-resolution data model and batch mapping flow
//! - Step 8: historical Binance 1s kline loading

mod binance_klines;
mod dashboard;
mod discovery;
mod observability;
mod slug;

pub use binance_klines::{
    load_1s_klines, plan_required_archives, sync_archives, ArchiveKind, ArchiveRef, BinanceSymbol,
    HistoricalKlinesConfig, Kline1s, KlineCoverageReport, KlineLoadError, KlineLoadRequest,
    KlineLoadResult, LocalArchive, LocalArchiveSource,
};
pub use dashboard::{
    apply_filters, build_display_snapshot, compute_in_interval, dashboard_router, demo_snapshot,
    format_row_for_display, market_link, render_dashboard_html, BetsOpenFilter,
    DashboardDisplayRow, DashboardDisplaySnapshot, DashboardFilters, DashboardQuery, DashboardRow,
    DashboardSnapshot, DashboardSnapshotSource, InIntervalFilter, InMemoryMockSnapshotSource,
    DASHBOARD_HEADERS,
};
#[cfg(feature = "discovery-sdk")]
pub use dashboard::{LiveDiscoveryConfig, LiveDiscoverySnapshotSource};
pub use discovery::resolve_discovery_batch_with_fetcher;
pub use discovery::{
    build_active_and_next_discovery_keys, build_active_discovery_keys,
    build_previous_active_and_next_discovery_keys, interval_starts_for_now, DiscoveryConfig,
    DiscoveryError, DiscoveryKey, DiscoveryRow, DiscoveryStatus, DiscoveryWindow, IntervalStarts,
    ScheduledDiscoveryKey, SlugFetchOutcome, UnresolvedReason, ALL_COINS, ALL_DURATIONS,
};
#[cfg(feature = "discovery-sdk")]
pub use discovery::{resolve_discovery_batch, SdkMarket};

pub use observability::{
    init_logging, log_app_bind, log_app_start, log_source_selected, logging_config_from_env,
    LogFormat, LoggingConfig, LoggingInitError,
};
pub use slug::{build_slug, parse_coin, parse_duration, Coin, Duration, SlugConfig, SlugError};

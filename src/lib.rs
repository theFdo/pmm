//! PMM core crate.
//!
//! Current implemented scope:
//! - Step 1: deterministic slug generation
//! - Step 2: discovery-resolution data model and batch mapping flow

mod discovery;
mod slug;

pub use discovery::resolve_discovery_batch_with_fetcher;
pub use discovery::{
    build_active_and_next_discovery_keys, build_active_discovery_keys,
    build_previous_active_and_next_discovery_keys, interval_ends_for_now, DiscoveryConfig,
    DiscoveryError, DiscoveryKey, DiscoveryRow, DiscoveryStatus, DiscoveryWindow, IntervalEnds,
    ScheduledDiscoveryKey, SlugFetchOutcome, UnresolvedReason, ALL_COINS, ALL_DURATIONS,
};
#[cfg(feature = "discovery-sdk")]
pub use discovery::{resolve_discovery_batch, SdkMarket};

pub use slug::{build_slug, parse_coin, parse_duration, Coin, Duration, SlugConfig, SlugError};

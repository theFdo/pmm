//! PMM core crate.
//!
//! Current implemented scope:
//! - Step 1: deterministic slug generation
//! - Step 2: discovery-resolution data model and batch mapping flow

mod discovery;
mod slug;

pub use discovery::resolve_discovery_batch_with_fetcher;
#[cfg(feature = "discovery-sdk")]
pub use discovery::{resolve_discovery_batch, SdkMarket};
pub use discovery::{
    DiscoveryConfig, DiscoveryError, DiscoveryKey, DiscoveryRow, DiscoveryStatus, SlugFetchOutcome,
    UnresolvedReason,
};

pub use slug::{build_slug, parse_coin, parse_duration, Coin, Duration, SlugConfig, SlugError};

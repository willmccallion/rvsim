//! Hardware Prefetcher implementations.
//!
//! This module contains the interface and implementations for various
//! hardware prefetchers used to hide memory latency.

pub mod next_line;

pub mod store_stream;

pub mod stream;

pub mod stride;

pub mod tagged;

pub use self::next_line::NextLinePrefetcher;
pub use self::store_stream::StoreStreamPrefetcher;
pub use self::stream::StreamPrefetcher;
pub use self::stride::StridePrefetcher;
pub use self::tagged::TaggedPrefetcher;

use crate::common::VirtAddr;

/// Trait for cache prefetcher implementations.
///
/// Prefetchers observe memory access patterns and generate prefetch
/// requests to reduce cache miss penalties.
pub trait Prefetcher: Send + Sync {
    /// Observes an access to `addr`, by the instruction at `pc` if it has
    /// one, that hit if `hit`, and returns the addresses to prefetch.
    fn observe(&mut self, addr: u64, pc: Option<VirtAddr>, hit: bool) -> Vec<u64>;
}

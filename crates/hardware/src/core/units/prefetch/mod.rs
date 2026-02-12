//! Hardware Prefetcher implementations.
//!
//! This module contains the interface and implementations for various
//! hardware prefetchers used to hide memory latency.

/// Next-line prefetcher (prefetches sequential cache lines).
pub mod next_line;

/// Stream prefetcher (detects ascending/descending access streams).
pub mod stream;

/// Stride prefetcher (detects constant-stride access patterns).
pub mod stride;

/// Tagged prefetcher (prefetches on demand misses and prefetch hits).
pub mod tagged;

pub use self::next_line::NextLinePrefetcher;
pub use self::stream::StreamPrefetcher;
pub use self::stride::StridePrefetcher;
pub use self::tagged::TaggedPrefetcher;

/// Trait for cache prefetcher implementations.
///
/// Prefetchers observe memory access patterns and generate prefetch
/// requests to reduce cache miss penalties.
pub trait Prefetcher: Send + Sync {
    /// Observes a memory access and generates prefetch addresses.
    ///
    /// Called by the cache on each access to allow the prefetcher to
    /// learn access patterns and generate prefetch requests.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address that was accessed
    /// * `hit` - Whether the access was a cache hit
    ///
    /// # Returns
    ///
    /// A vector of addresses to prefetch. Empty if no prefetches are needed.
    fn observe(&mut self, addr: u64, hit: bool) -> Vec<u64>;
}

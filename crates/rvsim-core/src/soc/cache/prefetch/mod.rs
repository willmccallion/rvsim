//! Hardware Prefetcher implementations.
//!
//! This module contains the interface and implementations for various
//! hardware prefetchers used to hide memory latency.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "prefetch candidates step over addresses by signed strides"
)]

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
    /// one, and appends the addresses to prefetch to `prefetches`.
    fn observe(
        &mut self,
        addr: u64,
        pc: Option<VirtAddr>,
        lookup: Lookup,
        prefetches: &mut Vec<u64>,
    );
}

/// Whether an access found its line in the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The line was there.
    Hit,
    /// The line was not.
    Miss,
}

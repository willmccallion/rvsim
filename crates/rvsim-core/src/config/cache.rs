//! The cache hierarchy: sizes, policies and prefetchers.

use super::defaults;
use super::prefetch::{LoadPrefetcherConfig, StorePrefetcherConfig};
use serde::Deserialize;
use std::num::NonZeroUsize;

/// Cache replacement policy algorithms.
///
/// Specifies the algorithm used to select which cache line to evict
/// when a new line must be installed in a full cache set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ReplacementPolicyKind {
    /// Least Recently Used replacement policy.
    ///
    /// Evicts the cache line that was accessed least recently.
    #[default]
    #[serde(alias = "Lru")]
    Lru,
    /// Pseudo-LRU (tree-based) replacement policy.
    ///
    /// Approximates LRU using a binary tree structure for lower
    /// hardware overhead while maintaining good performance.
    #[serde(alias = "Plru")]
    Plru,
    /// First In First Out replacement policy.
    ///
    /// Evicts the oldest cache line in the set (round-robin).
    #[serde(alias = "Fifo")]
    Fifo,
    /// Random replacement policy.
    ///
    /// Evicts a randomly selected cache line from the set.
    #[serde(alias = "Random")]
    Random,
    /// Most Recently Used replacement policy.
    ///
    /// Evicts the cache line that was accessed most recently.
    /// Effective for cyclic access patterns larger than the cache.
    #[serde(alias = "Mru")]
    Mru,
}

/// Cache inclusion policy for multi-level cache hierarchies.
///
/// Controls how evictions at one cache level interact with other levels
/// to maintain coherence within the same core's cache hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum InclusionPolicy {
    /// No Inclusion, Non-Exclusive (default).
    ///
    /// Cache levels operate independently. An eviction at one level does
    /// not affect other levels. This is the simplest policy and matches
    /// the existing behavior.
    #[default]
    #[serde(alias = "NINE")]
    Nine,
    /// Inclusive: L2 is a superset of L1.
    ///
    /// When a line is evicted from L2, the corresponding line in L1 is
    /// back-invalidated to prevent L1 from holding stale data.
    Inclusive,
    /// Exclusive: L1 and L2 hold disjoint sets of lines.
    ///
    /// When a line is evicted from L1, it is installed into L2 (swap policy).
    /// This maximizes effective cache capacity.
    Exclusive,
}

/// Hardware prefetcher types for cache prefetching.
///
/// Prefetchers predict future memory accesses and fetch data
/// into the cache before it is needed to reduce miss penalties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PrefetcherKind {
    /// No prefetching enabled.
    #[default]
    None,
    /// Next-line prefetcher.
    ///
    /// Prefetches the next sequential cache line after each access.
    NextLine,
    /// Stride prefetcher.
    ///
    /// Detects stride patterns in memory accesses and prefetches
    /// addresses following the detected stride.
    Stride,
    /// Stream prefetcher.
    ///
    /// Detects sequential stream direction (ascending/descending) and
    /// prefetches multiple lines ahead.
    Stream,
    /// Tagged prefetcher.
    ///
    /// Prefetches on demand misses and on hits to previously prefetched lines.
    Tagged,
}

/// Cache hierarchy configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheHierarchyConfig {
    /// L1 instruction cache
    pub l1_i: CacheConfig,
    /// L1 data cache
    pub l1_d: CacheConfig,
    /// Unified L2 cache
    pub l2: CacheConfig,
    /// Unified L3 cache (optional)
    pub l3: CacheConfig,
    /// Inclusion policy for the cache hierarchy
    #[serde(default)]
    pub inclusion_policy: InclusionPolicy,
    /// Number of Write Combining Buffer entries (0 = disabled)
    #[serde(default)]
    pub wcb_entries: usize,
    /// The load/store unit's load prefetcher.
    #[serde(default)]
    pub load_prefetcher: LoadPrefetcherConfig,
    /// The L1D's store-miss prefetcher.
    #[serde(default)]
    pub store_prefetcher: StorePrefetcherConfig,
}

/// Individual cache level configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// Enable this cache level
    #[serde(default)]
    pub enabled: bool,

    /// Total cache size in bytes
    #[serde(default = "CacheConfig::default_size")]
    pub size_bytes: usize,

    /// Cache line size in bytes
    #[serde(default = "CacheConfig::default_line")]
    pub line_bytes: usize,

    /// Associativity (number of ways)
    #[serde(default = "CacheConfig::default_ways")]
    pub ways: usize,

    /// Replacement policy
    #[serde(default)]
    pub policy: ReplacementPolicyKind,

    /// Access latency in cycles
    #[serde(default = "CacheConfig::default_latency")]
    pub latency: u64,

    /// Cycles from a line arriving from the next level to the requests
    /// waiting on it being answered: the fill is forwarded to them as it is
    /// written into the array (gem5's `response_latency`).
    #[serde(default = "CacheConfig::default_response_latency")]
    pub response_latency: u64,

    /// Hardware prefetcher type
    #[serde(default)]
    pub prefetcher: PrefetcherKind,

    /// Prefetcher table size (for stride prefetcher)
    #[serde(default = "CacheConfig::default_prefetch_table")]
    pub prefetch_table_size: usize,

    /// Prefetch degree (lines to prefetch per trigger)
    #[serde(default = "CacheConfig::default_prefetch_degree")]
    pub prefetch_degree: usize,

    /// Number of MSHRs (Miss Status Holding Registers): outstanding line
    /// fetches this level can have in flight. One gives a blocking cache.
    #[serde(default = "CacheConfig::default_mshr_count")]
    pub mshr_count: NonZeroUsize,

    /// Writeback buffer entries: victims in flight to the next level before
    /// the cache stops accepting requests.
    #[serde(default = "CacheConfig::default_write_buffers")]
    pub write_buffers: NonZeroUsize,

    /// Requests one MSHR can hold (gem5's `tgts_per_mshr`): once a line in
    /// flight has this many waiting, the cache accepts nothing until that
    /// line's fill returns.
    #[serde(default = "CacheConfig::default_targets_per_mshr")]
    pub targets_per_mshr: NonZeroUsize,
}

impl CacheConfig {
    /// Returns the default cache size in bytes.
    const fn default_size() -> usize {
        defaults::CACHE_SIZE
    }

    /// Returns the default cache line size in bytes.
    const fn default_line() -> usize {
        defaults::CACHE_LINE
    }

    /// Returns the default cache associativity (number of ways).
    const fn default_ways() -> usize {
        defaults::CACHE_WAYS
    }

    /// Returns the default cache access latency in cycles.
    const fn default_latency() -> u64 {
        defaults::CACHE_LATENCY
    }

    /// Returns the default fill-forwarding latency.
    const fn default_response_latency() -> u64 {
        defaults::CACHE_RESPONSE_LATENCY
    }

    /// Returns the default prefetcher pattern table size.
    const fn default_prefetch_table() -> usize {
        defaults::PREFETCH_TABLE_SIZE
    }

    /// Returns the default prefetch degree (lines per trigger).
    const fn default_prefetch_degree() -> usize {
        defaults::PREFETCH_DEGREE
    }

    /// Returns the default MSHR count.
    const fn default_mshr_count() -> NonZeroUsize {
        defaults::MSHR_COUNT
    }

    /// Returns the default writeback buffer size.
    const fn default_write_buffers() -> NonZeroUsize {
        defaults::WRITE_BUFFERS
    }

    /// Returns the default number of requests one MSHR can hold.
    const fn default_targets_per_mshr() -> NonZeroUsize {
        defaults::TARGETS_PER_MSHR
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            size_bytes: defaults::CACHE_SIZE,
            line_bytes: defaults::CACHE_LINE,
            ways: defaults::CACHE_WAYS,
            policy: ReplacementPolicyKind::default(),
            latency: defaults::CACHE_LATENCY,
            response_latency: defaults::CACHE_RESPONSE_LATENCY,
            prefetcher: PrefetcherKind::default(),
            prefetch_table_size: defaults::PREFETCH_TABLE_SIZE,
            prefetch_degree: defaults::PREFETCH_DEGREE,
            mshr_count: defaults::MSHR_COUNT,
            write_buffers: defaults::WRITE_BUFFERS,
            targets_per_mshr: defaults::TARGETS_PER_MSHR,
        }
    }
}

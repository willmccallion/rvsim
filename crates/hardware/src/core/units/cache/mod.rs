//! Set-Associative Cache Simulator.
//!
//! This module implements a configurable set-associative cache simulator.
//! It supports various replacement policies (LRU, FIFO, Random, etc.) and
//! hardware prefetchers. It models cache hits, misses, and write-back
//! penalties to simulate memory hierarchy latency.

/// Cache replacement policy implementations (FIFO, LRU, MRU, PLRU, Random).
pub mod policies;

use self::policies::{
    FifoPolicy, LruPolicy, MruPolicy, PlruPolicy, RandomPolicy, ReplacementPolicy,
};
use crate::config::{CacheConfig, Prefetcher as PrefetcherType, ReplacementPolicy as PolicyType};
use crate::core::units::prefetch::{
    NextLinePrefetcher, Prefetcher, StreamPrefetcher, StridePrefetcher, TaggedPrefetcher,
};

/// Cache line entry containing tag, validity, and dirty bits.
#[derive(Clone, Default)]
struct CacheLine {
    tag: u64,
    valid: bool,
    dirty: bool,
}

/// Cache simulator implementing a set-associative cache with configurable policies.
///
/// Supports various replacement policies (FIFO, LRU, PLRU, Random, MRU) and prefetchers
/// (Next-Line, Stride, Stream, Tagged). Models cache hits, misses, and write-back penalties.
pub struct CacheSim {
    /// Access latency in cycles (added on hit; miss adds next-level latency).
    pub latency: u64,
    /// When false, accesses bypass this cache and use next-level latency only.
    pub enabled: bool,
    /// Optional hardware prefetcher (boxed for dynamic dispatch; `Send + Sync` for thread safety).
    pub prefetcher: Option<Box<dyn Prefetcher + Send + Sync>>,
    lines: Vec<CacheLine>,
    num_sets: usize,
    ways: usize,
    line_bytes: usize,
    policy: Box<dyn ReplacementPolicy + Send + Sync>,
}

impl CacheSim {
    /// Creates a new cache simulator with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Cache configuration specifying size, associativity,
    ///   line size, replacement policy, and prefetcher
    ///
    /// # Returns
    ///
    /// A new `CacheSim` instance initialized according to the configuration.
    pub fn new(config: &CacheConfig) -> Self {
        let safe_ways = if config.ways == 0 { 1 } else { config.ways };
        let safe_line = if config.line_bytes == 0 {
            64
        } else {
            config.line_bytes
        };
        let safe_size = if config.size_bytes == 0 {
            4096
        } else {
            config.size_bytes
        };

        let num_lines = safe_size / safe_line;
        let num_sets = num_lines / safe_ways;

        let policy: Box<dyn ReplacementPolicy + Send + Sync> = match config.policy {
            PolicyType::Fifo => Box::new(FifoPolicy::new(num_sets, safe_ways)),
            PolicyType::Random => Box::new(RandomPolicy::new(num_sets, safe_ways)),
            PolicyType::Plru => Box::new(PlruPolicy::new(num_sets, safe_ways)),
            PolicyType::Lru => Box::new(LruPolicy::new(num_sets, safe_ways)),
            PolicyType::Mru => Box::new(MruPolicy::new(num_sets, safe_ways)),
        };

        let prefetcher: Option<Box<dyn Prefetcher + Send + Sync>> = match config.prefetcher {
            PrefetcherType::NextLine => Some(Box::new(NextLinePrefetcher::new(
                safe_line,
                config.prefetch_degree,
            ))),
            PrefetcherType::Stride => Some(Box::new(StridePrefetcher::new(
                safe_line,
                config.prefetch_table_size,
                config.prefetch_degree,
            ))),
            PrefetcherType::Stream => Some(Box::new(StreamPrefetcher::new(
                safe_line,
                config.prefetch_degree,
            ))),
            PrefetcherType::Tagged => Some(Box::new(TaggedPrefetcher::new(
                safe_line,
                config.prefetch_degree,
            ))),
            PrefetcherType::None => None,
        };

        Self {
            lines: vec![CacheLine::default(); num_sets * safe_ways],
            num_sets,
            ways: safe_ways,
            line_bytes: safe_line,
            latency: config.latency,
            enabled: config.enabled,
            policy,
            prefetcher,
        }
    }

    /// Checks if the cache contains the specified address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address to check
    ///
    /// # Returns
    ///
    /// `true` if the address is present in the cache, `false` otherwise.
    ///
    /// # Panics
    ///
    /// This function will not panic. Array indexing is guaranteed safe because:
    /// - `set_index` is always `< num_sets` (modulo operation)
    /// - `base_idx = set_index * ways` is always `< lines.len()`
    /// - `idx = base_idx + i` where `i < ways` ensures `idx < lines.len()`
    pub fn contains(&self, addr: u64) -> bool {
        if !self.enabled {
            return false;
        }

        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        for i in 0..self.ways {
            let idx = base_idx + i;
            if self.lines[idx].valid && self.lines[idx].tag == tag {
                return true;
            }
        }
        false
    }

    /// Installs a cache line for the specified address.
    ///
    /// Selects a victim line using the replacement policy and installs
    /// the new line. Returns the penalty for write-back if the victim
    /// line was dirty.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address to install
    /// * `is_write` - Whether this is a write operation
    /// * `next_level_latency` - Latency of the next cache level (for write-back penalty)
    ///
    /// # Returns
    ///
    /// The penalty in cycles for writing back a dirty victim line.
    fn install_line(&mut self, addr: u64, is_write: bool, next_level_latency: u64) -> u64 {
        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        let victim_way = self.policy.get_victim(set_index);
        let victim_idx = base_idx + victim_way;
        let mut penalty = 0;

        if self.lines[victim_idx].valid && self.lines[victim_idx].dirty {
            penalty += next_level_latency;
        }

        self.lines[victim_idx] = CacheLine {
            tag,
            valid: true,
            dirty: is_write,
        };
        self.policy.update(set_index, victim_way);

        penalty
    }

    /// Accesses the cache for the specified address.
    ///
    /// Performs a cache lookup, updates replacement policy on hit,
    /// installs the line on miss, and triggers prefetcher. Returns
    /// hit status and penalty cycles.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address to access
    /// * `is_write` - Whether this is a write operation
    /// * `next_level_latency` - Latency of the next cache level
    ///
    /// # Returns
    ///
    /// A tuple `(hit, penalty)` where `hit` indicates a cache hit
    /// and `penalty` is the number of penalty cycles (0 on hit,
    /// miss penalty + write-back penalty on miss).
    pub fn access(&mut self, addr: u64, is_write: bool, next_level_latency: u64) -> (bool, u64) {
        if !self.enabled {
            return (false, 0);
        }

        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        let mut hit = false;
        let mut penalty = 0;

        for i in 0..self.ways {
            let idx = base_idx + i;
            if self.lines[idx].valid && self.lines[idx].tag == tag {
                self.policy.update(set_index, i);
                if is_write {
                    self.lines[idx].dirty = true;
                }
                hit = true;
                break;
            }
        }

        if !hit {
            penalty += self.install_line(addr, is_write, next_level_latency);
        }

        let mut prefetches = Vec::new();
        if let Some(ref mut pref) = self.prefetcher {
            prefetches = pref.observe(addr, hit);
        }

        for target in prefetches {
            if !self.contains(target) {
                self.install_line(target, false, next_level_latency);
            }
        }

        (hit, penalty)
    }

    /// Flushes all dirty cache lines, invalidating them.
    ///
    /// Marks all valid and dirty lines as invalid. Used for cache
    /// coherence operations and system calls that require cache flushing.
    pub fn flush(&mut self) {
        if !self.enabled {
            return;
        }
        for line in &mut self.lines {
            if line.valid && line.dirty {
                line.dirty = false;
                line.valid = false;
            }
        }
    }
}

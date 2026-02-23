//! Set-Associative Cache Simulator.
//!
//! This module implements a configurable set-associative cache simulator.
//! It supports various replacement policies (LRU, FIFO, Random, etc.) and
//! hardware prefetchers. It models cache hits, misses, and write-back
//! penalties to simulate memory hierarchy latency.

/// Cache replacement policy implementations (FIFO, LRU, MRU, PLRU, Random).
pub mod policies;

/// Miss Status Holding Registers (MSHRs) for non-blocking cache access.
pub mod mshr;

use self::policies::{
    FifoPolicy, LruPolicy, MruPolicy, PlruPolicy, RandomPolicy, ReplacementPolicy,
};
use crate::config::{CacheConfig, Prefetcher as PrefetcherType, ReplacementPolicy as PolicyType};
use crate::core::units::prefetch::{
    NextLinePrefetcher, Prefetcher, StreamPrefetcher, StridePrefetcher, TaggedPrefetcher,
};

/// Information about an evicted cache line.
#[derive(Clone, Copy, Debug)]
pub struct EvictedLine {
    /// Physical address of the evicted line (cache-line aligned).
    pub addr: u64,
    /// Whether the evicted line was dirty.
    pub dirty: bool,
}

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

    /// Reconstructs the physical address from a set index and tag.
    #[inline]
    fn reconstruct_addr(&self, set_index: usize, tag: u64) -> u64 {
        tag * (self.line_bytes * self.num_sets) as u64 + (set_index * self.line_bytes) as u64
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
        self.install_line_tracked(addr, is_write, next_level_latency)
            .0
    }

    /// Installs a cache line and returns both the penalty and eviction info.
    ///
    /// Returns `(penalty, Option<EvictedLine>)`. The evicted line info is
    /// `Some` when a valid line was replaced, enabling the caller to implement
    /// inclusion/exclusion policies.
    fn install_line_tracked(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (u64, Option<EvictedLine>) {
        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        let victim_way = self.policy.get_victim(set_index);
        let victim_idx = base_idx + victim_way;
        let mut penalty = 0;
        let mut evicted = None;

        if self.lines[victim_idx].valid {
            let victim_addr = self.reconstruct_addr(set_index, self.lines[victim_idx].tag);
            let victim_dirty = self.lines[victim_idx].dirty;
            evicted = Some(EvictedLine {
                addr: victim_addr,
                dirty: victim_dirty,
            });
            if victim_dirty {
                penalty += next_level_latency;
            }
        }

        self.lines[victim_idx] = CacheLine {
            tag,
            valid: true,
            dirty: is_write,
        };
        self.policy.update(set_index, victim_way);

        (penalty, evicted)
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

    /// Accesses the cache with eviction tracking for inclusion/exclusion policies.
    ///
    /// Returns `(hit, penalty, Vec<EvictedLine>)` where the evicted lines
    /// include both the demand miss eviction and any prefetch-triggered evictions.
    ///
    /// Prefetch candidates are installed directly. Use `access_tracked_split` if
    /// you need to filter prefetch candidates before installation.
    pub fn access_tracked(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (bool, u64, Vec<EvictedLine>) {
        let (hit, penalty, evictions, prefetch_candidates) =
            self.access_tracked_split(addr, is_write, next_level_latency);

        // Install all prefetch candidates directly
        let mut all_evictions = evictions;
        for target in prefetch_candidates {
            if !self.contains(target) {
                let (_pen, evicted) = self.install_line_tracked(target, false, next_level_latency);
                if let Some(ev) = evicted {
                    all_evictions.push(ev);
                }
            }
        }

        (hit, penalty, all_evictions)
    }

    /// Accesses the cache with eviction tracking, returning prefetch candidates
    /// separately instead of installing them.
    ///
    /// Returns `(hit, penalty, demand_evictions, prefetch_candidates)`.
    /// The caller is responsible for filtering and installing the prefetch candidates.
    pub fn access_tracked_split(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (bool, u64, Vec<EvictedLine>, Vec<u64>) {
        if !self.enabled {
            return (false, 0, Vec::new(), Vec::new());
        }

        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        let mut hit = false;
        let mut penalty = 0;
        let mut evictions = Vec::new();

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
            let (pen, evicted) = self.install_line_tracked(addr, is_write, next_level_latency);
            penalty += pen;
            if let Some(ev) = evicted {
                evictions.push(ev);
            }
        }

        let mut prefetches = Vec::new();
        if let Some(ref mut pref) = self.prefetcher {
            prefetches = pref.observe(addr, hit);
        }

        (hit, penalty, evictions, prefetches)
    }

    /// Installs prefetch targets into this cache, returning any evictions.
    ///
    /// Used after filtering prefetch candidates through a shared prefetch filter.
    pub fn install_prefetches(
        &mut self,
        targets: &[u64],
        next_level_latency: u64,
    ) -> Vec<EvictedLine> {
        let mut evictions = Vec::new();
        for &target in targets {
            if !self.contains(target) {
                let (_pen, evicted) = self.install_line_tracked(target, false, next_level_latency);
                if let Some(ev) = evicted {
                    evictions.push(ev);
                }
            }
        }
        evictions
    }

    /// Non-blocking cache access: checks for hit/miss without installing the line on miss.
    ///
    /// On hit: updates replacement policy and dirty bit, triggers prefetcher. Returns true.
    /// On miss: triggers prefetcher but does NOT install the line. Returns false.
    /// The caller (MSHR) is responsible for installing the line later.
    pub fn access_check(&mut self, addr: u64, is_write: bool) -> bool {
        if !self.enabled {
            return false;
        }

        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        let mut hit = false;
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

        let mut prefetches = Vec::new();
        if let Some(ref mut pref) = self.prefetcher {
            prefetches = pref.observe(addr, hit);
        }
        for target in prefetches {
            if !self.contains(target) {
                self.install_line(target, false, 0);
            }
        }

        hit
    }

    /// Install a cache line from outside (e.g. when an MSHR completes).
    ///
    /// Returns the write-back penalty if the evicted victim was dirty.
    pub fn install_line_public(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> u64 {
        self.install_line(addr, is_write, next_level_latency)
    }

    /// Install a cache line from outside with eviction tracking.
    ///
    /// Returns `(penalty, Option<EvictedLine>)`.
    pub fn install_line_public_tracked(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (u64, Option<EvictedLine>) {
        self.install_line_tracked(addr, is_write, next_level_latency)
    }

    /// Invalidates the cache line containing the specified address.
    ///
    /// Used by the inclusive cache policy to back-invalidate L1 when L2 evicts a line.
    /// Returns true if the line was found and invalidated, false if not present.
    pub fn invalidate_line(&mut self, addr: u64) -> bool {
        if !self.enabled {
            return false;
        }

        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        for i in 0..self.ways {
            let idx = base_idx + i;
            if self.lines[idx].valid && self.lines[idx].tag == tag {
                self.lines[idx].valid = false;
                self.lines[idx].dirty = false;
                return true;
            }
        }
        false
    }

    /// Installs a line without evicting the previous one (used for exclusive policy
    /// when swapping an L1 evictee into L2, if there is an invalid way available).
    /// Falls back to normal install_line if no free way exists.
    ///
    /// Returns `(penalty, Option<EvictedLine>)`.
    pub fn install_or_replace(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (u64, Option<EvictedLine>) {
        if !self.enabled {
            return (0, None);
        }

        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;

        // Try to find an invalid (free) way first
        for i in 0..self.ways {
            let idx = base_idx + i;
            if !self.lines[idx].valid {
                self.lines[idx] = CacheLine {
                    tag,
                    valid: true,
                    dirty: is_write,
                };
                self.policy.update(set_index, i);
                return (0, None);
            }
        }

        // No free way — fall back to replacement
        self.install_line_tracked(addr, is_write, next_level_latency)
    }

    /// Returns the cache line size in bytes.
    #[inline]
    pub fn line_bytes(&self) -> usize {
        self.line_bytes
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

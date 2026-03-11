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

use std::simd::cmp::SimdPartialEq;
use std::simd::{Mask, Simd};

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

/// Cache simulator implementing a set-associative cache with configurable policies.
///
/// Supports various replacement policies (FIFO, LRU, PLRU, Random, MRU) and prefetchers
/// (Next-Line, Stride, Stream, Tagged). Models cache hits, misses, and write-back penalties.
///
/// ## Internal layout — Structure of Arrays
///
/// Tags, valid bits, and dirty bits are stored in three separate flat arrays rather than
/// a single `Vec<CacheLine>`.  For a cache with `S` sets and `W` ways, entry `(set, way)`
/// lives at index `set * W + way` in each array.  This layout lets the tag-scan loop read
/// all tags for one set as a contiguous slice of `u64`, enabling SIMD parallel comparison.
pub struct CacheSim {
    /// Access latency in cycles (added on hit; miss adds next-level latency).
    pub latency: u64,
    /// When false, accesses bypass this cache and use next-level latency only.
    pub enabled: bool,
    /// Optional hardware prefetcher (boxed for dynamic dispatch; `Send + Sync` for thread safety).
    pub prefetcher: Option<Box<dyn Prefetcher + Send + Sync>>,

    // ── SoA line storage ──────────────────────────────────────────────────────
    tags: Vec<u64>,
    valid: Vec<u8>, // 0 = invalid, 1 = valid  (u8 avoids bool padding)
    dirty: Vec<u8>, // 0 = clean,   1 = dirty

    num_sets: usize,
    ways: usize,

    // Precomputed shift amounts (cache sizes are always powers of two).
    // set_index = (addr >> line_shift) & set_mask
    // tag       =  addr >> (line_shift + set_shift)
    line_shift: u32,
    set_shift: u32,
    set_mask: u64,

    policy: Box<dyn ReplacementPolicy + Send + Sync>,
}

impl std::fmt::Debug for CacheSim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheSim")
            .field("latency", &self.latency)
            .field("enabled", &self.enabled)
            .field("num_sets", &self.num_sets)
            .field("ways", &self.ways)
            .field("line_bytes", &(1usize << self.line_shift))
            .finish_non_exhaustive()
    }
}

impl CacheSim {
    /// Creates a new cache simulator with the specified configuration.
    pub fn new(config: &CacheConfig) -> Self {
        let safe_ways = if config.ways == 0 { 1 } else { config.ways };
        let safe_line = if config.line_bytes == 0 { 64 } else { config.line_bytes };
        let safe_size = if config.size_bytes == 0 { 4096 } else { config.size_bytes };

        let num_lines = safe_size / safe_line;
        let num_sets = num_lines / safe_ways;

        // Round up to the nearest power of two so shifts work correctly.
        let line_bytes = safe_line.next_power_of_two();
        let num_sets_p2 = num_sets.next_power_of_two();

        let line_shift = line_bytes.trailing_zeros();
        let set_shift = num_sets_p2.trailing_zeros();
        let set_mask = (num_sets_p2 as u64) - 1;

        let policy: Box<dyn ReplacementPolicy + Send + Sync> = match config.policy {
            PolicyType::Fifo => Box::new(FifoPolicy::new(num_sets_p2, safe_ways)),
            PolicyType::Random => Box::new(RandomPolicy::new(num_sets_p2, safe_ways)),
            PolicyType::Plru => Box::new(PlruPolicy::new(num_sets_p2, safe_ways)),
            PolicyType::Lru => Box::new(LruPolicy::new(num_sets_p2, safe_ways)),
            PolicyType::Mru => Box::new(MruPolicy::new(num_sets_p2, safe_ways)),
        };

        let prefetcher: Option<Box<dyn Prefetcher + Send + Sync>> = match config.prefetcher {
            PrefetcherType::NextLine => {
                Some(Box::new(NextLinePrefetcher::new(line_bytes, config.prefetch_degree)))
            }
            PrefetcherType::Stride => Some(Box::new(StridePrefetcher::new(
                line_bytes,
                config.prefetch_table_size,
                config.prefetch_degree,
            ))),
            PrefetcherType::Stream => {
                Some(Box::new(StreamPrefetcher::new(line_bytes, config.prefetch_degree)))
            }
            PrefetcherType::Tagged => {
                Some(Box::new(TaggedPrefetcher::new(line_bytes, config.prefetch_degree)))
            }
            PrefetcherType::None => None,
        };

        let total = num_sets_p2 * safe_ways;
        Self {
            tags: vec![0u64; total],
            valid: vec![0u8; total],
            dirty: vec![0u8; total],
            num_sets: num_sets_p2,
            ways: safe_ways,
            line_shift,
            set_shift,
            set_mask,
            latency: config.latency,
            enabled: config.enabled,
            policy,
            prefetcher,
        }
    }

    // ── Address decomposition ─────────────────────────────────────────────────

    #[inline(always)]
    fn decompose(&self, addr: u64) -> (usize, u64) {
        let line_addr = addr >> self.line_shift;
        let set_index = (line_addr & self.set_mask) as usize;
        let tag = line_addr >> self.set_shift;
        (set_index, tag)
    }

    #[inline(always)]
    const fn reconstruct_addr(&self, set_index: usize, tag: u64) -> u64 {
        ((tag << self.set_shift) | (set_index as u64)) << self.line_shift
    }

    // ── Tag scan ──────────────────────────────────────────────────────────────

    /// Returns the way index of a hit, or `None` on miss.
    ///
    /// Uses 4-wide or 8-wide SIMD tag comparison when the way count aligns,
    /// falling back to a scalar loop otherwise.
    #[inline(always)]
    fn find_way(&self, base: usize, tag: u64) -> Option<usize> {
        let tags = &self.tags[base..base + self.ways];
        let valid = &self.valid[base..base + self.ways];

        match self.ways {
            4 => {
                let tv = Simd::<u64, 4>::from_slice(tags);
                let needle = Simd::<u64, 4>::splat(tag);
                let tag_match: Mask<i64, 4> = tv.simd_eq(needle);

                let vv = Simd::<u8, 4>::from_slice(valid);
                let one = Simd::<u8, 4>::splat(1);
                let valid_mask: Mask<i8, 4> = vv.simd_eq(one);

                // Widen valid mask to i64 lanes to AND with tag_match.
                let valid_wide = Mask::<i64, 4>::from_array(valid_mask.to_array());
                let hit = tag_match & valid_wide;
                hit.first_set()
            }
            8 => {
                let tv = Simd::<u64, 8>::from_slice(tags);
                let needle = Simd::<u64, 8>::splat(tag);
                let tag_match: Mask<i64, 8> = tv.simd_eq(needle);

                let vv = Simd::<u8, 8>::from_slice(valid);
                let one = Simd::<u8, 8>::splat(1);
                let valid_mask: Mask<i8, 8> = vv.simd_eq(one);

                let valid_wide = Mask::<i64, 8>::from_array(valid_mask.to_array());
                let hit = tag_match & valid_wide;
                hit.first_set()
            }
            _ => {
                for i in 0..self.ways {
                    if valid[i] != 0 && tags[i] == tag {
                        return Some(i);
                    }
                }
                None
            }
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Checks if the cache contains the specified address.
    pub fn contains(&self, addr: u64) -> bool {
        if !self.enabled {
            return false;
        }
        let (set_index, tag) = self.decompose(addr);
        self.find_way(set_index * self.ways, tag).is_some()
    }

    /// Installs a cache line for the specified address.
    fn install_line(&mut self, addr: u64, is_write: bool, next_level_latency: u64) -> u64 {
        self.install_line_tracked(addr, is_write, next_level_latency).0
    }

    /// Installs a cache line and returns both the penalty and eviction info.
    fn install_line_tracked(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (u64, Option<EvictedLine>) {
        let (set_index, tag) = self.decompose(addr);
        let base = set_index * self.ways;

        let victim_way = self.policy.get_victim(set_index);
        let victim_idx = base + victim_way;
        let mut penalty = 0;
        let mut evicted = None;

        if self.valid[victim_idx] != 0 {
            let victim_addr = self.reconstruct_addr(set_index, self.tags[victim_idx]);
            let victim_dirty = self.dirty[victim_idx] != 0;
            evicted = Some(EvictedLine { addr: victim_addr, dirty: victim_dirty });
            if victim_dirty {
                penalty += next_level_latency;
            }
        }

        self.tags[victim_idx] = tag;
        self.valid[victim_idx] = 1;
        self.dirty[victim_idx] = u8::from(is_write);
        self.policy.update(set_index, victim_way);

        (penalty, evicted)
    }

    /// Accesses the cache for the specified address.
    ///
    /// Performs a cache lookup, updates replacement policy on hit,
    /// installs the line on miss, and triggers prefetcher. Returns
    /// hit status and penalty cycles.
    pub fn access(&mut self, addr: u64, is_write: bool, next_level_latency: u64) -> (bool, u64) {
        if !self.enabled {
            return (false, 0);
        }

        let (set_index, tag) = self.decompose(addr);
        let base = set_index * self.ways;

        let mut penalty = 0;
        let hit = if let Some(way) = self.find_way(base, tag) {
            self.policy.update(set_index, way);
            if is_write {
                self.dirty[base + way] = 1;
            }
            true
        } else {
            penalty += self.install_line(addr, is_write, next_level_latency);
            false
        };

        let prefetches =
            self.prefetcher.as_mut().map_or_else(Vec::new, |pref| pref.observe(addr, hit));

        for target in prefetches {
            if !self.contains(target) {
                let _ = self.install_line(target, false, next_level_latency);
            }
        }

        (hit, penalty)
    }

    /// Accesses the cache with eviction tracking for inclusion/exclusion policies.
    pub fn access_tracked(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (bool, u64, Vec<EvictedLine>) {
        let (hit, penalty, evictions, prefetch_candidates) =
            self.access_tracked_split(addr, is_write, next_level_latency);

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

    /// Accesses the cache with eviction tracking, returning prefetch candidates separately.
    pub fn access_tracked_split(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (bool, u64, Vec<EvictedLine>, Vec<u64>) {
        if !self.enabled {
            return (false, 0, Vec::new(), Vec::new());
        }

        let (set_index, tag) = self.decompose(addr);
        let base = set_index * self.ways;

        let mut penalty = 0;
        let mut evictions = Vec::new();

        let hit = if let Some(way) = self.find_way(base, tag) {
            self.policy.update(set_index, way);
            if is_write {
                self.dirty[base + way] = 1;
            }
            true
        } else {
            let (pen, evicted) = self.install_line_tracked(addr, is_write, next_level_latency);
            penalty += pen;
            if let Some(ev) = evicted {
                evictions.push(ev);
            }
            false
        };

        let prefetches =
            self.prefetcher.as_mut().map_or_else(Vec::new, |pref| pref.observe(addr, hit));

        (hit, penalty, evictions, prefetches)
    }

    /// Installs prefetch targets into this cache, returning any evictions.
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
    pub fn access_check(&mut self, addr: u64, is_write: bool) -> bool {
        if !self.enabled {
            return false;
        }

        let (set_index, tag) = self.decompose(addr);
        let base = set_index * self.ways;

        let hit = if let Some(way) = self.find_way(base, tag) {
            self.policy.update(set_index, way);
            if is_write {
                self.dirty[base + way] = 1;
            }
            true
        } else {
            false
        };

        let prefetches =
            self.prefetcher.as_mut().map_or_else(Vec::new, |pref| pref.observe(addr, hit));
        for target in prefetches {
            if !self.contains(target) {
                let _ = self.install_line(target, false, 0);
            }
        }

        hit
    }

    /// Install a cache line from outside (e.g. when an MSHR completes).
    pub fn install_line_public(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> u64 {
        self.install_line(addr, is_write, next_level_latency)
    }

    /// Install a cache line from outside with eviction tracking.
    pub fn install_line_public_tracked(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (u64, Option<EvictedLine>) {
        self.install_line_tracked(addr, is_write, next_level_latency)
    }

    /// Invalidates the cache line containing the specified address.
    pub fn invalidate_line(&mut self, addr: u64) -> bool {
        if !self.enabled {
            return false;
        }

        let (set_index, tag) = self.decompose(addr);
        let base = set_index * self.ways;

        if let Some(way) = self.find_way(base, tag) {
            let idx = base + way;
            self.valid[idx] = 0;
            self.dirty[idx] = 0;
            return true;
        }
        false
    }

    /// Installs a line without evicting the previous one if a free way exists.
    pub fn install_or_replace(
        &mut self,
        addr: u64,
        is_write: bool,
        next_level_latency: u64,
    ) -> (u64, Option<EvictedLine>) {
        if !self.enabled {
            return (0, None);
        }

        let (set_index, tag) = self.decompose(addr);
        let base = set_index * self.ways;

        // Try to find an invalid (free) way first.
        for i in 0..self.ways {
            if self.valid[base + i] == 0 {
                self.tags[base + i] = tag;
                self.valid[base + i] = 1;
                self.dirty[base + i] = u8::from(is_write);
                self.policy.update(set_index, i);
                return (0, None);
            }
        }

        self.install_line_tracked(addr, is_write, next_level_latency)
    }

    /// Returns the cache line size in bytes.
    #[inline]
    pub const fn line_bytes(&self) -> usize {
        1 << self.line_shift
    }

    /// Flushes the cache: writes back all dirty lines and invalidates all entries.
    pub fn flush(&mut self) -> Vec<EvictedLine> {
        let mut evicted = Vec::new();
        if !self.enabled {
            return evicted;
        }
        for i in 0..self.tags.len() {
            if self.valid[i] != 0 && self.dirty[i] != 0 {
                let set_index = i / self.ways;
                evicted.push(EvictedLine {
                    addr: self.reconstruct_addr(set_index, self.tags[i]),
                    dirty: true,
                });
                self.dirty[i] = 0;
                self.valid[i] = 0;
            }
        }
        evicted
    }

    /// Invalidates all cache lines (dirty and clean), returning evicted dirty lines.
    pub fn invalidate_all(&mut self) -> Vec<EvictedLine> {
        let mut evicted = Vec::new();
        if !self.enabled {
            return evicted;
        }
        for i in 0..self.tags.len() {
            if self.valid[i] != 0 {
                if self.dirty[i] != 0 {
                    let set_index = i / self.ways;
                    evicted.push(EvictedLine {
                        addr: self.reconstruct_addr(set_index, self.tags[i]),
                        dirty: true,
                    });
                }
                self.dirty[i] = 0;
                self.valid[i] = 0;
            }
        }
        evicted
    }
}

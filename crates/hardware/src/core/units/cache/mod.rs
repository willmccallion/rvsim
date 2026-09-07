//! Set-associative cache.
//!
//! Each cache level is its own [`Handle`] component. Requests arrive as
//! [`Packet::MemReq`]; on a hit the cache schedules a [`Packet::MemResp`] back
//! to the requester, on a miss it forwards a `MemReq` downstream and records
//! the original requester in [`Cache::pending`] so the eventual response can
//! be routed back. Evictions emit [`Packet::CacheInval`] to upstream caches
//! to maintain inclusion.

pub mod policies;

pub mod mshr;

use std::collections::HashMap;

use self::policies::{
    FifoPolicy, LruPolicy, MruPolicy, PlruPolicy, RandomPolicy, ReplacementPolicy,
};
use crate::common::{LineAddr, PhysAddr, VirtAddr};
use crate::config::{CacheConfig, Prefetcher as PrefetcherType, ReplacementPolicy as PolicyType};
use crate::core::units::prefetch::{
    NextLinePrefetcher, Prefetcher, StreamPrefetcher, StridePrefetcher, TaggedPrefetcher,
};
use crate::sim::components::{CacheId, ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, CacheLevel, HitLevel, MemOp, MemRespData, Packet};

/// Information about an evicted cache line.
#[derive(Clone, Copy, Debug)]
pub struct EvictedLine {
    /// Physical address of the evicted line (cache-line aligned).
    pub addr: u64,
    /// Whether the evicted line was dirty.
    pub dirty: bool,
}

/// In-flight memory request the cache is waiting to satisfy.
///
/// One entry per [`ReqId`] forwarded downstream; the eventual `MemResp` is
/// routed back to `source`.
#[derive(Clone, Debug)]
pub struct PendingRequest {
    /// Component that issued the original [`Packet::MemReq`].
    pub source: ComponentId,
    /// Post-translation address.
    pub paddr: PhysAddr,
    /// Pre-translation address for fault reporting.
    pub vaddr: Option<VirtAddr>,
    /// Access width.
    pub size: AccessSize,
    /// Read / write / atomic / fetch.
    pub op: MemOp,
}

/// Cache line entry containing tag, validity, and dirty bits.
#[derive(Clone, Debug, Default)]
struct CacheLine {
    tag: u64,
    valid: bool,
    dirty: bool,
}

/// A set-associative cache at one level of the memory hierarchy.
pub struct Cache {
    /// Arena-relative identifier; the `ComponentId` form is `ComponentId::Cache(id)`.
    pub id: CacheId,
    /// Position in the hierarchy.
    pub level: CacheLevel,
    /// Caches that may hold a copy of any line installed here; receive
    /// `CacheInval` on eviction to maintain inclusion.
    pub upstream: Vec<ComponentId>,
    /// Where to forward a `MemReq` on a miss. `None` for the last cache
    /// when no downstream has been wired yet.
    pub downstream: Option<ComponentId>,
    /// Outstanding misses waiting on a downstream response.
    pub pending: HashMap<ReqId, PendingRequest>,
    /// Access latency in cycles.
    pub latency: u64,
    /// When false, accesses bypass this cache and forward straight downstream.
    pub enabled: bool,
    /// Optional hardware prefetcher.
    pub prefetcher: Option<Box<dyn Prefetcher + Send + Sync>>,
    lines: Vec<CacheLine>,
    num_sets: usize,
    ways: usize,
    line_bytes: usize,
    policy: Box<dyn ReplacementPolicy + Send + Sync>,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache")
            .field("id", &self.id)
            .field("level", &self.level)
            .field("latency", &self.latency)
            .field("enabled", &self.enabled)
            .field("num_sets", &self.num_sets)
            .field("ways", &self.ways)
            .field("line_bytes", &self.line_bytes)
            .finish_non_exhaustive()
    }
}

impl Cache {
    /// Creates a new cache.
    ///
    /// `id` and `level` identify the cache for routing; `upstream` and
    /// `downstream` are empty by default and configured by the system builder.
    pub fn new(id: CacheId, level: CacheLevel, config: &CacheConfig) -> Self {
        let safe_ways = if config.ways == 0 { 1 } else { config.ways };
        let safe_line = if config.line_bytes == 0 { 64 } else { config.line_bytes };
        let safe_size = if config.size_bytes == 0 { 4096 } else { config.size_bytes };

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
            PrefetcherType::NextLine => {
                Some(Box::new(NextLinePrefetcher::new(safe_line, config.prefetch_degree)))
            }
            PrefetcherType::Stride => Some(Box::new(StridePrefetcher::new(
                safe_line,
                config.prefetch_table_size,
                config.prefetch_degree,
            ))),
            PrefetcherType::Stream => {
                Some(Box::new(StreamPrefetcher::new(safe_line, config.prefetch_degree)))
            }
            PrefetcherType::Tagged => {
                Some(Box::new(TaggedPrefetcher::new(safe_line, config.prefetch_degree)))
            }
            PrefetcherType::None => None,
        };

        Self {
            id,
            level,
            upstream: Vec::new(),
            downstream: None,
            pending: HashMap::new(),
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

    /// Sets the downstream target for forwarded misses.
    pub const fn set_downstream(&mut self, downstream: ComponentId) {
        self.downstream = Some(downstream);
    }

    /// Adds an upstream consumer that should receive `CacheInval` on
    /// eviction to keep inclusion invariants.
    pub fn add_upstream(&mut self, upstream: ComponentId) {
        self.upstream.push(upstream);
    }

    /// Reconstructs the physical address from a set index and tag.
    #[inline]
    const fn reconstruct_addr(&self, set_index: usize, tag: u64) -> u64 {
        tag * (self.line_bytes * self.num_sets) as u64 + (set_index * self.line_bytes) as u64
    }

    /// Returns true if the cache holds the line containing `addr`.
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

    /// Installs a cache line. Returns the write-back penalty when the victim
    /// was dirty.
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
            evicted = Some(EvictedLine { addr: victim_addr, dirty: victim_dirty });
            if victim_dirty {
                penalty += next_level_latency;
            }
        }

        self.lines[victim_idx] = CacheLine { tag, valid: true, dirty: is_write };
        self.policy.update(set_index, victim_way);

        (penalty, evicted)
    }

    /// Probe the cache, install on miss, run the prefetcher. Returns `(hit, penalty)`.
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

        let prefetches =
            self.prefetcher.as_mut().map_or_else(Vec::new, |pref| pref.observe(addr, hit));

        for target in prefetches {
            if !self.contains(target) {
                let _ = self.install_line(target, false, next_level_latency);
            }
        }

        (hit, penalty)
    }

    /// Probe the cache with eviction tracking. Prefetch candidates are
    /// installed directly. Use `access_tracked_split` to filter them first.
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

    /// Probe with eviction tracking, returning prefetch candidates separately
    /// instead of installing them.
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

        let prefetches =
            self.prefetcher.as_mut().map_or_else(Vec::new, |pref| pref.observe(addr, hit));

        (hit, penalty, evictions, prefetches)
    }

    /// Installs prefetch targets, returning any evictions.
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

    /// Non-blocking probe: checks for hit/miss without installing on miss.
    ///
    /// On hit, updates replacement policy and dirty bit, triggers prefetcher.
    /// On miss, triggers prefetcher but does NOT install the line.
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

    /// Writes back the line at `addr` if dirty and clears the dirty bit.
    /// Used by Zicbom `cbo.clean`. Returns true if the line was present.
    pub fn clean_line(&mut self, addr: u64) -> bool {
        if !self.enabled {
            return false;
        }
        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;
        let base_idx = set_index * self.ways;
        for i in 0..self.ways {
            let idx = base_idx + i;
            if self.lines[idx].valid && self.lines[idx].tag == tag {
                self.lines[idx].dirty = false;
                return true;
            }
        }
        false
    }

    /// Invalidates the line containing `addr`. Returns true if the line was present.
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

    /// Installs a line into an invalid way without eviction if possible.
    /// Falls back to `install_line_tracked` if no free way exists.
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

        for i in 0..self.ways {
            let idx = base_idx + i;
            if !self.lines[idx].valid {
                self.lines[idx] = CacheLine { tag, valid: true, dirty: is_write };
                self.policy.update(set_index, i);
                return (0, None);
            }
        }

        self.install_line_tracked(addr, is_write, next_level_latency)
    }

    /// Returns the cache line size in bytes.
    #[inline]
    pub const fn line_bytes(&self) -> usize {
        self.line_bytes
    }

    /// Writes back all dirty lines and invalidates them. Returns the evicted
    /// dirty lines for writeback accounting; clean lines remain valid.
    pub fn flush(&mut self) -> Vec<EvictedLine> {
        let mut evicted = Vec::new();
        if !self.enabled {
            return evicted;
        }
        for i in 0..self.lines.len() {
            if self.lines[i].valid && self.lines[i].dirty {
                let set_index = i / self.ways;
                evicted.push(EvictedLine {
                    addr: self.reconstruct_addr(set_index, self.lines[i].tag),
                    dirty: true,
                });
                self.lines[i].dirty = false;
                self.lines[i].valid = false;
            }
        }
        evicted
    }

    /// Invalidates every line, returning evicted dirty lines. Used for I-cache
    /// invalidation on FENCE.I where stale clean lines must also be discarded.
    pub fn invalidate_all(&mut self) -> Vec<EvictedLine> {
        let mut evicted = Vec::new();
        if !self.enabled {
            return evicted;
        }
        for i in 0..self.lines.len() {
            if self.lines[i].valid {
                if self.lines[i].dirty {
                    let set_index = i / self.ways;
                    evicted.push(EvictedLine {
                        addr: self.reconstruct_addr(set_index, self.lines[i].tag),
                        dirty: true,
                    });
                }
                self.lines[i].dirty = false;
                self.lines[i].valid = false;
            }
        }
        evicted
    }
}

const fn hit_level_for(level: CacheLevel) -> HitLevel {
    match level {
        CacheLevel::L1I | CacheLevel::L1D => HitLevel::L1,
        CacheLevel::L2 => HitLevel::L2,
        CacheLevel::L3 => HitLevel::L3,
    }
}

impl Handle for Cache {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        match packet {
            Packet::MemReq { req_id, paddr, vaddr, size, op } => {
                let line_addr = LineAddr::from_phys(paddr, self.line_bytes as u64);

                if !self.enabled {
                    self.forward_pass_through(req_id, paddr, vaddr, size, op, source, ctx);
                    return;
                }

                let is_write = matches!(op, MemOp::Write { .. });
                let hit = self.access_check(paddr.val(), is_write);

                if hit {
                    ctx.scheduler.schedule(
                        ctx.cycle + self.latency,
                        source,
                        ctx.self_id,
                        Packet::MemResp {
                            req_id,
                            line_addr,
                            data: MemRespData::Small(0),
                            hit_level: hit_level_for(self.level),
                        },
                    );
                } else if let Some(ds) = self.downstream {
                    let _ = self.pending.insert(
                        req_id,
                        PendingRequest { source, paddr, vaddr, size, op: op.clone() },
                    );
                    ctx.scheduler.schedule(
                        ctx.cycle + self.latency,
                        ds,
                        ctx.self_id,
                        Packet::MemReq { req_id, paddr, vaddr, size, op },
                    );
                }
            }
            Packet::MemResp { req_id, line_addr, data, hit_level } => {
                let Some(pending) = self.pending.remove(&req_id) else { return };

                if self.enabled {
                    let is_write = matches!(pending.op, MemOp::Write { .. });
                    let (_pen, evicted) =
                        self.install_line_tracked(pending.paddr.val(), is_write, 0);

                    if let Some(ev) = evicted {
                        let ev_line =
                            LineAddr::from_phys(PhysAddr::new(ev.addr), self.line_bytes as u64);
                        for &u in &self.upstream {
                            ctx.scheduler.schedule(
                                ctx.cycle,
                                u,
                                ctx.self_id,
                                Packet::CacheInval { line_addr: ev_line },
                            );
                        }
                    }
                }

                ctx.scheduler.schedule(
                    ctx.cycle,
                    pending.source,
                    ctx.self_id,
                    Packet::MemResp { req_id, line_addr, data, hit_level },
                );
            }
            Packet::CacheInval { line_addr } => {
                let _ = self.invalidate_line(line_addr.val());
            }
            Packet::CacheClean { line_addr } => {
                let _ = self.clean_line(line_addr.val());
            }
            _ => {}
        }
    }
}

impl Cache {
    /// Pass-through routing used when this cache level is disabled: forwards
    /// the request to `downstream` with zero added latency while recording the
    /// original requester so the eventual response routes back through here.
    // Mirrors the shape of a MemReq packet plus routing metadata; splitting into
    // a struct would just re-inline the same fields at every call site.
    #[allow(clippy::too_many_arguments)]
    fn forward_pass_through(
        &mut self,
        req_id: ReqId,
        paddr: PhysAddr,
        vaddr: Option<VirtAddr>,
        size: AccessSize,
        op: MemOp,
        source: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(ds) = self.downstream else { return };
        let _ = self.pending.insert(
            req_id,
            PendingRequest { source, paddr, vaddr, size, op: op.clone() },
        );
        ctx.scheduler.schedule(
            ctx.cycle,
            ds,
            ctx.self_id,
            Packet::MemReq { req_id, paddr, vaddr, size, op },
        );
    }
}

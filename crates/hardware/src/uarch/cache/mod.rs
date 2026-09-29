//! Set-associative cache with MSHRs and a writeback buffer.
//!
//! Each cache level is one [`Handle`] component. A request arrives as
//! [`Packet::MemReq`]: a hit answers after the access latency; a miss
//! allocates an MSHR, or joins the one already fetching the line, and sends
//! one line request downstream. The fill installs the line, writes a dirty
//! victim back through the writeback buffer, and answers every request the
//! MSHR gathered. While the MSHRs or the writeback buffer are full the
//! cache is blocked: new requests queue and are retried in arrival order as
//! entries free up, which is how a blocked port stalls its requester.
//!
//! Caches hold tags and states, never data: functional bytes live in RAM.

pub mod mshr;
pub mod policies;
pub mod stats;
pub mod writeback_buffer;

mod coherence;
mod inclusion;
mod request;
mod writeback;

use std::collections::VecDeque;

use self::mshr::MshrTable;
use self::policies::{
    FifoPolicy, LruPolicy, MruPolicy, PlruPolicy, RandomPolicy, ReplacementPolicy,
};
use self::stats::CacheStatPaths;
use self::writeback_buffer::WritebackBuffer;
use crate::common::{CoreId, LineAddr, PhysAddr, VirtAddr};
use crate::config::{CacheConfig, InclusionPolicy, PrefetcherKind, ReplacementPolicyKind};
use crate::sim::components::{CacheId, ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, CacheLevel, MemOp, MesiState, Packet, ProbeKind};
use crate::uarch::prefetch::{
    NextLinePrefetcher, Prefetcher, StreamPrefetcher, StridePrefetcher, TaggedPrefetcher,
};

/// One tag-array entry.
#[derive(Clone, Copy, Debug, Default)]
struct CacheLine {
    tag: u64,
    state: MesiState,
    /// Bit `i` set: `upstream[i]` was given this line and has not told us
    /// it dropped it. Probes and back-invalidations go only to those.
    upper: u8,
}

impl CacheLine {
    const fn valid(self) -> bool {
        !matches!(self.state, MesiState::Invalid)
    }

    const fn dirty(self) -> bool {
        matches!(self.state, MesiState::Modified | MesiState::Owned)
    }
}

/// A request that arrived while the cache was blocked.
#[derive(Clone, Debug)]
struct BlockedRequest {
    source: ComponentId,
    req_id: ReqId,
    paddr: PhysAddr,
    vaddr: Option<VirtAddr>,
    size: AccessSize,
    op: MemOp,
}

/// A request forwarded downstream without a line of our own (the level is
/// disabled, or a writeback for a line we do not hold), remembered so the
/// response can be routed back to its requester.
#[derive(Clone, Copy, Debug)]
struct Forwarded {
    ours: ReqId,
    source: ComponentId,
    theirs: ReqId,
    line: LineAddr,
}

/// Who asked for a line's rights and how to answer them.
#[derive(Clone, Copy, Debug)]
enum ProbeOrigin {
    /// A lower private cache's [`Packet::Probe`].
    Probe {
        /// Who probed us.
        from: ComponentId,
        /// Their correlator.
        txn: ReqId,
    },
    /// The home agent's snoop, answered with a `SnoopResp`.
    Snoop {
        /// The home's transaction.
        txn: ReqId,
    },
}

/// A probe or snoop this cache forwarded to the caches above it and has
/// not yet answered.
#[derive(Clone, Copy, Debug)]
struct PendingProbe {
    /// Correlator we gave the forwarded probes.
    ours: ReqId,
    origin: ProbeOrigin,
    line: LineAddr,
    kind: ProbeKind,
    /// Upstream answers still outstanding.
    remaining: usize,
    /// Whether any copy (ours or an upper one) was dirty.
    dirty: bool,
    /// Whether we held the line at all.
    had_copy: bool,
}

/// A set-associative cache at one level of the memory hierarchy.
pub struct Cache {
    /// Arena-relative identifier; the `ComponentId` form is `ComponentId::Cache(id)`.
    pub id: CacheId,
    /// Position in the hierarchy.
    pub level: CacheLevel,
    /// Caches above this one that may hold copies of its lines.
    pub upstream: Vec<ComponentId>,
    /// Where misses and writebacks go. `None` for the last cache when no
    /// downstream has been wired yet.
    pub downstream: Option<ComponentId>,
    /// Access latency in cycles.
    pub latency: u64,
    /// Cycles from a fill arriving to the requests waiting on it being
    /// answered.
    response_latency: u64,
    /// When false, accesses bypass this cache and forward straight downstream.
    pub enabled: bool,
    /// Optional hardware prefetcher.
    pub prefetcher: Option<Box<dyn Prefetcher + Send + Sync>>,
    /// Stat paths rooted at this cache's subject.
    pub stat_paths: CacheStatPaths,
    /// Relationship with the caches above this one.
    upstream_inclusion: InclusionPolicy,
    /// True when clean victims are handed to the next level (this cache is
    /// the upper half of an exclusive pair).
    clean_victims_to_downstream: bool,
    /// Set when this cache is a core's requesting agent on the coherence
    /// fabric: misses, writebacks and evictions become coherence messages
    /// and snoops arrive from the home.
    coherent: Option<CoreId>,
    lines: Vec<CacheLine>,
    num_sets: usize,
    ways: usize,
    line_bytes: usize,
    policy: Box<dyn ReplacementPolicy + Send + Sync>,
    mshrs: MshrTable,
    targets_per_mshr: usize,
    /// The MSHR that reached `targets_per_mshr`, blocking the cache until
    /// its fill returns (gem5's `noTargetMSHR`).
    full_mshr: Option<ReqId>,
    writebacks: WritebackBuffer,
    blocked: VecDeque<BlockedRequest>,
    forwarded: Vec<Forwarded>,
    /// Maintenance operations waiting for a fetch of their line to fill.
    after_fill: Vec<BlockedRequest>,
    pending_probes: Vec<PendingProbe>,
    next_req: u64,
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
            .field("mshrs", &self.mshrs)
            .field("writebacks", &self.writebacks)
            .finish_non_exhaustive()
    }
}

impl Cache {
    /// Creates a cache whose stats live under `stat_subject`.
    ///
    /// `id` and `level` identify the cache for routing; `upstream`,
    /// `downstream` and the inclusion relationship are configured by the
    /// system builder.
    pub fn new(id: CacheId, level: CacheLevel, config: &CacheConfig, stat_subject: &str) -> Self {
        let safe_ways = if config.ways == 0 { 1 } else { config.ways };
        let safe_line = if config.line_bytes == 0 { 64 } else { config.line_bytes };
        let safe_size = if config.size_bytes == 0 { 4096 } else { config.size_bytes };

        let num_lines = safe_size / safe_line;
        let num_sets = (num_lines / safe_ways).max(1);

        let policy: Box<dyn ReplacementPolicy + Send + Sync> = match config.policy {
            ReplacementPolicyKind::Fifo => Box::new(FifoPolicy::new(num_sets, safe_ways)),
            ReplacementPolicyKind::Random => Box::new(RandomPolicy::new(num_sets, safe_ways)),
            ReplacementPolicyKind::Plru => Box::new(PlruPolicy::new(num_sets, safe_ways)),
            ReplacementPolicyKind::Lru => Box::new(LruPolicy::new(num_sets, safe_ways)),
            ReplacementPolicyKind::Mru => Box::new(MruPolicy::new(num_sets, safe_ways)),
        };

        let prefetcher: Option<Box<dyn Prefetcher + Send + Sync>> = match config.prefetcher {
            PrefetcherKind::NextLine => {
                Some(Box::new(NextLinePrefetcher::new(safe_line, config.prefetch_degree)))
            }
            PrefetcherKind::Stride => Some(Box::new(StridePrefetcher::new(
                safe_line,
                config.prefetch_table_size,
                config.prefetch_degree,
            ))),
            PrefetcherKind::Stream => {
                Some(Box::new(StreamPrefetcher::new(safe_line, config.prefetch_degree)))
            }
            PrefetcherKind::Tagged => {
                Some(Box::new(TaggedPrefetcher::new(safe_line, config.prefetch_degree)))
            }
            PrefetcherKind::None => None,
        };

        Self {
            id,
            level,
            upstream: Vec::new(),
            downstream: None,
            latency: config.latency,
            response_latency: config.response_latency,
            enabled: config.enabled,
            prefetcher,
            stat_paths: CacheStatPaths::new(stat_subject),
            upstream_inclusion: InclusionPolicy::Nine,
            clean_victims_to_downstream: false,
            coherent: None,
            lines: vec![CacheLine::default(); num_sets * safe_ways],
            num_sets,
            ways: safe_ways,
            line_bytes: safe_line,
            policy,
            mshrs: MshrTable::new(config.mshr_count),
            targets_per_mshr: config.targets_per_mshr.max(1),
            full_mshr: None,
            writebacks: WritebackBuffer::new(config.write_buffers),
            blocked: VecDeque::new(),
            forwarded: Vec::new(),
            after_fill: Vec::new(),
            pending_probes: Vec::new(),
            next_req: 0,
        }
    }

    /// Sets the downstream target for misses and writebacks.
    pub const fn set_downstream(&mut self, downstream: ComponentId) {
        self.downstream = Some(downstream);
    }

    /// Adds a cache above this one.
    pub fn add_upstream(&mut self, upstream: ComponentId) {
        self.upstream.push(upstream);
    }

    /// Sets how this cache treats the caches above it: `Inclusive` back-
    /// invalidates them on eviction, `Exclusive` gives up its own copy when
    /// it fills one of them, `Nine` does neither.
    pub const fn set_upstream_inclusion(&mut self, policy: InclusionPolicy) {
        self.upstream_inclusion = policy;
    }

    /// Makes this cache hand clean victims to the next level (the upper
    /// half of an exclusive pair).
    pub const fn set_clean_victims_to_downstream(&mut self, enabled: bool) {
        self.clean_victims_to_downstream = enabled;
    }

    /// Makes this cache `core`'s requesting agent on the coherence fabric.
    pub const fn set_coherent(&mut self, core: CoreId) {
        self.coherent = Some(core);
    }

    /// The core this cache requests on behalf of, when coherent.
    #[must_use]
    pub const fn coherent_core(&self) -> Option<CoreId> {
        self.coherent
    }

    /// Lines with a fetch, writeback, probe or snoop in progress.
    #[must_use]
    pub fn lines_in_flight(&self) -> Vec<LineAddr> {
        self.mshrs
            .iter()
            .map(|m| m.line)
            .chain(self.writebacks.lines())
            .chain(self.pending_probes.iter().map(|p| p.line))
            .chain(self.forwarded.iter().map(|f| f.line))
            .collect()
    }

    /// True when this cache keeps lines (a disabled cache only passes
    /// requests through).
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns the cache line size in bytes.
    #[inline]
    pub const fn line_bytes(&self) -> usize {
        self.line_bytes
    }

    /// Lines this cache can hold.
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Outstanding line fetches.
    #[must_use]
    pub const fn mshrs(&self) -> &MshrTable {
        &self.mshrs
    }

    /// Writebacks in flight to the next level.
    #[must_use]
    pub const fn writebacks(&self) -> &WritebackBuffer {
        &self.writebacks
    }

    /// Requests waiting for the cache to unblock.
    #[must_use]
    pub fn blocked_requests(&self) -> usize {
        self.blocked.len()
    }

    /// Every valid line with its state, for audits.
    #[must_use]
    pub fn held_lines(&self) -> Vec<(LineAddr, MesiState)> {
        let mut held = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if line.valid() {
                let set_index = index / self.ways;
                held.push((self.line_of(self.reconstruct_addr(set_index, line.tag)), line.state));
            }
        }
        held
    }

    /// Lines that appear in more than one way of a set. Always empty for a
    /// correct cache; audited by tests and the coherence checker.
    #[must_use]
    pub fn duplicate_lines(&self) -> Vec<LineAddr> {
        let mut duplicates = Vec::new();
        for set_index in 0..self.num_sets {
            let set = &self.lines[set_index * self.ways..(set_index + 1) * self.ways];
            for (i, line) in set.iter().enumerate() {
                if line.valid()
                    && set[..i].iter().any(|other| other.valid() && other.tag == line.tag)
                {
                    duplicates.push(self.line_of(self.reconstruct_addr(set_index, line.tag)));
                }
            }
        }
        duplicates
    }

    const fn line_of(&self, addr: u64) -> LineAddr {
        LineAddr::from_phys(PhysAddr::new(addr), self.line_bytes as u64)
    }

    const fn set_index(&self, addr: u64) -> usize {
        ((addr as usize) / self.line_bytes) % self.num_sets
    }

    const fn tag_of(&self, addr: u64) -> u64 {
        addr / (self.line_bytes * self.num_sets) as u64
    }

    const fn reconstruct_addr(&self, set_index: usize, tag: u64) -> u64 {
        tag * (self.line_bytes * self.num_sets) as u64 + (set_index * self.line_bytes) as u64
    }

    fn find_way(&self, addr: u64) -> Option<usize> {
        let set_index = self.set_index(addr);
        let tag = self.tag_of(addr);
        (0..self.ways).find(|&way| {
            let line = self.lines[set_index * self.ways + way];
            line.valid() && line.tag == tag
        })
    }

    /// Returns true if the cache holds the line containing `addr`.
    pub fn contains(&self, addr: u64) -> bool {
        self.enabled && self.find_way(addr).is_some()
    }

    /// Makes our copy of the line containing `addr` clean, keeping it.
    /// Returns whether it was dirty.
    fn clean_line(&mut self, addr: u64) -> bool {
        let Some(way) = self.find_way(addr) else { return false };
        let index = self.set_index(addr) * self.ways + way;
        let was_dirty = self.lines[index].dirty();
        if was_dirty {
            self.lines[index].state = MesiState::Exclusive;
        }
        was_dirty
    }

    /// Drops our copy of the line containing `addr`. Returns whether it was
    /// dirty.
    fn invalidate_line(&mut self, addr: u64) -> bool {
        let Some(way) = self.find_way(addr) else { return false };
        let index = self.set_index(addr) * self.ways + way;
        let was_dirty = self.lines[index].dirty();
        self.lines[index].state = MesiState::Invalid;
        was_dirty
    }

    /// Drops every line, as an instruction cache does for FENCE.I.
    pub fn invalidate_all(&mut self) {
        for line in &mut self.lines {
            line.state = MesiState::Invalid;
        }
    }
}

impl Handle for Cache {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        match packet {
            Packet::MemReq { req_id, paddr, vaddr, size, op } => {
                self.on_request(BlockedRequest { source, req_id, paddr, vaddr, size, op }, ctx);
            }
            Packet::MemResp { req_id, line_addr, data, hit_level, state } => {
                self.on_response(req_id, line_addr, data, hit_level, state, ctx);
            }
            Packet::Probe { line_addr, kind, txn } => {
                self.on_probe(line_addr, kind, txn, source, ctx);
            }
            Packet::ProbeResp { txn, had_copy, dirty, .. } => {
                self.on_probe_resp(txn, had_copy, dirty, source, ctx);
            }
            Packet::Coh(msg) => self.on_coherence(msg, ctx),
            Packet::CacheInval { line_addr } => self.on_back_invalidate(line_addr, ctx),
            _ => {}
        }
    }
}

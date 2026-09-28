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

use std::collections::VecDeque;

use self::mshr::{Mshr, MshrTable, MshrTarget};
use self::policies::{
    FifoPolicy, LruPolicy, MruPolicy, PlruPolicy, RandomPolicy, ReplacementPolicy,
};
use self::stats::CacheStatPaths;
use self::writeback_buffer::{Writeback, WritebackBuffer};
use crate::coherence::messages::{CoherenceMsg, ReqKind, SnoopKind};
use crate::common::{CoreId, LineAddr, PhysAddr, VirtAddr};
use crate::config::{
    CacheConfig, InclusionPolicy, Prefetcher as PrefetcherType, ReplacementPolicy as PolicyType,
};
use crate::core::units::prefetch::{
    NextLinePrefetcher, Prefetcher, StreamPrefetcher, StridePrefetcher, TaggedPrefetcher,
};
use crate::sim::components::{CacheId, ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{
    AccessSize, CacheLevel, HitLevel, Maintenance, MemOp, MemRespData, MesiState, Packet, ProbeKind,
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

    const fn alloc_req_id(&mut self) -> ReqId {
        let seq = self.next_req;
        self.next_req = seq.wrapping_add(1);
        ReqId::for_cache(self.id, seq)
    }

    const fn is_blocked(&self) -> bool {
        self.mshrs.is_full() || self.full_mshr.is_some() || self.writebacks.is_full()
    }

    const fn hit_level(&self) -> HitLevel {
        match self.level {
            CacheLevel::L1I | CacheLevel::L1D => HitLevel::L1,
            CacheLevel::L2 => HitLevel::L2,
            CacheLevel::L3 => HitLevel::L3,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn respond(
        &self,
        ctx: &mut HandleCtx<'_>,
        target: ComponentId,
        req_id: ReqId,
        paddr: PhysAddr,
        at: u64,
        hit_level: HitLevel,
        state: MesiState,
        data: MemRespData,
    ) {
        ctx.scheduler.schedule(
            at,
            target,
            ctx.self_id,
            Packet::MemResp {
                req_id,
                line_addr: self.line_of(paddr.val()),
                data,
                hit_level,
                state,
            },
        );
    }

    /// Makes a hart's access take effect as this cache serves it and returns
    /// what it read; line traffic from the level above only moves permission.
    fn serve(
        paddr: PhysAddr,
        size: AccessSize,
        op: &MemOp,
        ctx: &mut HandleCtx<'_>,
    ) -> MemRespData {
        if op.takes_effect_when_served(size) {
            ctx.memory.perform(paddr, size, op)
        } else {
            MemRespData::Small(0)
        }
    }

    fn on_request(&mut self, req: BlockedRequest, ctx: &mut HandleCtx<'_>) {
        if !self.enabled {
            if self.coherent.is_some() && req.size == AccessSize::Line {
                self.forward_as_coherence_request(&req, ctx);
            } else {
                self.forward(req, ctx);
            }
            return;
        }
        if self.is_blocked() {
            ctx.stats.counter(self.stat_paths.blocked_requests).inc();
            self.blocked.push_back(req);
            return;
        }
        if let MemOp::Writeback { dirty } = req.op {
            self.on_writeback(&req, dirty, ctx);
            return;
        }
        if let MemOp::Maintain { op, dirty } = req.op {
            self.on_maintain(req, op, dirty, ctx);
            return;
        }

        let addr = req.paddr.val();
        // An atomic is performed in the cache: it needs the line writable
        // and leaves it modified, like a store.
        let is_write =
            matches!(req.op, MemOp::Write { .. } | MemOp::ReadOwn | MemOp::Atomic { .. });
        let set_index = self.set_index(addr);
        let present = self.find_way(addr);
        let needs_permission = is_write
            && present.is_some_and(|way| {
                self.lines[set_index * self.ways + way].state == MesiState::Shared
            });
        if let Some(way) = present.filter(|_| !needs_permission) {
            self.policy.update(set_index, way);
            let index = set_index * self.ways + way;
            if is_write {
                self.lines[index].state = MesiState::Modified;
            }
            ctx.stats.counter(self.stat_paths.hits).inc();
            let hit_level = self.hit_level();
            let granted = self.lines[index].state;
            let data = Self::serve(req.paddr, req.size, &req.op, ctx);
            self.respond(
                ctx,
                req.source,
                req.req_id,
                req.paddr,
                ctx.cycle + self.latency,
                hit_level,
                granted,
                data,
            );
            self.note_upper_copy(index, req.source);
            if self.upstream_inclusion == InclusionPolicy::Exclusive
                && matches!(req.source, ComponentId::Cache(_))
            {
                // The upper level now owns the line.
                self.lines[set_index * self.ways + way].state = MesiState::Invalid;
            }
            self.observe_prefetcher(addr, true, ctx);
            return;
        }

        ctx.stats.counter(self.stat_paths.misses).inc();
        let line = self.line_of(addr);
        let target = MshrTarget {
            source: req.source,
            req_id: req.req_id,
            paddr: req.paddr,
            vaddr: req.vaddr,
            size: req.size,
            op: req.op,
        };
        if let Some(mshr) = self.mshrs.find_line_mut(line) {
            ctx.stats.counter(self.stat_paths.mshr_hits).inc();
            if mshr.prefetch && mshr.targets.is_empty() && mshr.deferred.is_empty() {
                ctx.stats.counter(self.stat_paths.prefetches_useful).inc();
            }
            if (is_write && !mshr.write) || !mshr.deferred.is_empty() {
                mshr.deferred.push(target);
            } else {
                mshr.targets.push(target);
            }
            if mshr.target_count() >= self.targets_per_mshr {
                self.full_mshr = Some(mshr.req_id);
            }
        } else {
            let fetch_op = match target.op {
                MemOp::Fetch => MemOp::Fetch,
                _ if is_write => MemOp::ReadOwn,
                _ => MemOp::Read,
            };
            let vaddr = target.vaddr;
            self.start_fetch(line, vec![target], is_write, false, fetch_op, vaddr, ctx);
        }
        self.observe_prefetcher(addr, false, ctx);
    }

    /// Allocates an MSHR for `line` and sends the line request downstream
    /// after the tag lookup.
    #[allow(clippy::too_many_arguments)]
    fn start_fetch(
        &mut self,
        line: LineAddr,
        targets: Vec<MshrTarget>,
        write: bool,
        prefetch: bool,
        op: MemOp,
        vaddr: Option<VirtAddr>,
        ctx: &mut HandleCtx<'_>,
    ) {
        let req_id = self.alloc_req_id();
        let upgrade = self.find_way(line.val()).is_some();
        self.mshrs.allocate(Mshr {
            line,
            req_id,
            targets,
            deferred: Vec::new(),
            write,
            prefetch,
            issued_at: ctx.cycle,
            upgrade,
        });
        let Some(downstream) = self.downstream else { return };
        let packet = match self.coherent {
            Some(requester) => {
                let kind = if upgrade {
                    ctx.stats.counter(self.stat_paths.upgrades).inc();
                    ReqKind::CleanUnique
                } else if write {
                    ReqKind::ReadUnique
                } else {
                    ReqKind::ReadShared
                };
                Packet::Coh(CoherenceMsg::Req { txn: req_id, line, kind, requester })
            }
            None => {
                Packet::MemReq { req_id, paddr: line.phys(), vaddr, size: AccessSize::Line, op }
            }
        };
        ctx.scheduler.schedule(ctx.cycle + self.latency, downstream, ctx.self_id, packet);
    }

    /// Runs the prefetcher on a demand access and starts fetches for the
    /// lines it wants that are neither present nor already in flight,
    /// keeping one MSHR free for demand misses.
    fn observe_prefetcher(&mut self, addr: u64, hit: bool, ctx: &mut HandleCtx<'_>) {
        let Some(prefetcher) = self.prefetcher.as_mut() else { return };
        let candidates = prefetcher.observe(addr, hit);
        for candidate in candidates {
            if self.mshrs.free() <= 1 || self.downstream.is_none() {
                return;
            }
            let line = self.line_of(candidate);
            if self.contains(candidate) || self.mshrs.holds(line) || self.writebacks.holds(line) {
                continue;
            }
            ctx.stats.counter(self.stat_paths.prefetches_issued).inc();
            self.start_fetch(line, Vec::new(), false, true, MemOp::Read, None, ctx);
        }
    }

    /// A whole line arriving from above: merge into our copy when we hold
    /// it, otherwise pass it on without allocating. The requester is
    /// acknowledged after the access latency either way.
    fn on_writeback(&mut self, req: &BlockedRequest, dirty: bool, ctx: &mut HandleCtx<'_>) {
        let addr = req.paddr.val();
        let hit_level = self.hit_level();
        self.respond(
            ctx,
            req.source,
            req.req_id,
            req.paddr,
            ctx.cycle + self.latency,
            hit_level,
            MesiState::Invalid,
            MemRespData::Small(0),
        );
        let line = self.line_of(addr);
        // A clean writeback is an eviction notice. A dirty one may also be
        // a probed cache keeping a shared copy, so its presence bit stays
        // until a probe finds it gone.
        if !dirty {
            self.forget_upper_copy(addr, req.source);
        }
        if self.pending_probes.iter().any(|p| p.line == line) {
            self.note_probe_writeback(line, dirty, ctx);
            return;
        }
        if let Some(way) = self.find_way(addr) {
            let index = self.set_index(addr) * self.ways + way;
            if dirty {
                self.lines[index].state = MesiState::Modified;
            }
            return;
        }
        if dirty || self.clean_victims_to_downstream {
            // Not ours: forward downstream through the writeback buffer.
            self.write_back(self.line_of(addr), dirty, ctx);
        }
    }

    /// Sends a line to the next level and tracks it until acknowledged.
    fn write_back(&mut self, line: LineAddr, dirty: bool, ctx: &mut HandleCtx<'_>) {
        let Some(downstream) = self.downstream else { return };
        let req_id = self.alloc_req_id();
        self.writebacks.allocate(Writeback { line, req_id, dirty });
        ctx.stats.counter(self.stat_paths.writebacks).inc();
        let packet = self.coherent.map_or_else(
            || Packet::MemReq {
                req_id,
                paddr: line.phys(),
                vaddr: None,
                size: AccessSize::Line,
                op: MemOp::Writeback { dirty },
            },
            |requester| {
                Packet::Coh(CoherenceMsg::Req {
                    txn: req_id,
                    line,
                    kind: ReqKind::WriteBack { dirty },
                    requester,
                })
            },
        );
        ctx.scheduler.schedule(ctx.cycle + 1, downstream, ctx.self_id, packet);
    }

    /// Tells the home a clean line was dropped, so its tracking stays exact.
    fn notify_evict(&mut self, line: LineAddr, ctx: &mut HandleCtx<'_>) {
        let (Some(requester), Some(downstream)) = (self.coherent, self.downstream) else { return };
        let txn = self.alloc_req_id();
        ctx.scheduler.schedule(
            ctx.cycle + 1,
            downstream,
            ctx.self_id,
            Packet::Coh(CoherenceMsg::Req { txn, line, kind: ReqKind::Evict, requester }),
        );
    }

    /// Pass-through used when this level is disabled: forward the request
    /// with our own correlator and remember where the response goes.
    fn forward(&mut self, req: BlockedRequest, ctx: &mut HandleCtx<'_>) {
        let Some(downstream) = self.downstream else { return };
        let ours = self.alloc_req_id();
        let line = self.line_of(req.paddr.val());
        self.forwarded.push(Forwarded { ours, source: req.source, theirs: req.req_id, line });
        ctx.scheduler.schedule(
            ctx.cycle,
            downstream,
            ctx.self_id,
            Packet::MemReq {
                req_id: ours,
                paddr: req.paddr,
                vaddr: req.vaddr,
                size: req.size,
                op: req.op,
            },
        );
    }

    /// A maintenance operation from above: applied to our copy once any
    /// fetch of the line has filled, then passed on toward memory with our
    /// dirty data, as gem5's cache always forwards one.
    fn on_maintain(
        &mut self,
        req: BlockedRequest,
        op: Maintenance,
        dirty_above: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let line = self.line_of(req.paddr.val());
        if self.mshrs.holds(line) {
            self.after_fill.push(req);
            return;
        }
        ctx.stats.counter(self.stat_paths.maintenance).inc();
        let dirty_here = self.apply_maintenance(line, op, req.source, ctx);
        let dirty = (dirty_above || dirty_here) && op != Maintenance::Invalidate;
        let at = ctx.cycle + self.latency;
        let Some(downstream) = self.downstream else {
            let level = self.hit_level();
            let ack = MemRespData::Small(0);
            self.respond(
                ctx,
                req.source,
                req.req_id,
                req.paddr,
                at,
                level,
                MesiState::Invalid,
                ack,
            );
            return;
        };
        let ours = self.alloc_req_id();
        self.forwarded.push(Forwarded { ours, source: req.source, theirs: req.req_id, line });
        let packet = self.coherent.map_or_else(
            || Packet::MemReq {
                req_id: ours,
                paddr: line.phys(),
                vaddr: None,
                size: AccessSize::Line,
                op: MemOp::Maintain { op, dirty },
            },
            |requester| {
                let kind = ReqKind::Maintain { op, dirty };
                Packet::Coh(CoherenceMsg::Req { txn: ours, line, kind, requester })
            },
        );
        ctx.scheduler.schedule(at, downstream, ctx.self_id, packet);
    }

    /// Applies a maintenance operation to our copy of `line`, dropping the
    /// copies above other than the requester's for a flush or invalidate.
    /// Returns whether our copy was dirty.
    fn apply_maintenance(
        &mut self,
        line: LineAddr,
        op: Maintenance,
        requester: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) -> bool {
        match op {
            Maintenance::Clean => self.clean_line(line.val()),
            Maintenance::Flush | Maintenance::Invalidate => {
                let holders: Vec<ComponentId> =
                    self.upper_holders(line).into_iter().filter(|&h| h != requester).collect();
                self.back_invalidate(line, &holders, ctx);
                self.invalidate_line(line.val())
            }
        }
    }

    /// A disabled cache that is still its core's requesting agent: the
    /// caches above it hold the lines, so their line requests, writebacks
    /// and evictions are spoken to the home on their behalf.
    fn forward_as_coherence_request(&mut self, req: &BlockedRequest, ctx: &mut HandleCtx<'_>) {
        let (Some(requester), Some(downstream)) = (self.coherent, self.downstream) else { return };
        let line = self.line_of(req.paddr.val());
        let kind = match &req.op {
            MemOp::Writeback { dirty } => {
                let dirty = *dirty;
                self.respond(
                    ctx,
                    req.source,
                    req.req_id,
                    req.paddr,
                    ctx.cycle + self.latency,
                    self.hit_level(),
                    MesiState::Invalid,
                    MemRespData::Small(0),
                );
                if self.pending_probes.iter().any(|p| p.line == line) {
                    self.note_probe_writeback(line, dirty, ctx);
                } else if dirty {
                    self.write_back(line, true, ctx);
                } else {
                    self.notify_evict(line, ctx);
                }
                return;
            }
            MemOp::Write { .. } | MemOp::ReadOwn | MemOp::Atomic { .. } => ReqKind::ReadUnique,
            MemOp::Read | MemOp::Fetch => ReqKind::ReadShared,
            MemOp::Maintain { op, dirty } => ReqKind::Maintain { op: *op, dirty: *dirty },
        };
        let ours = self.alloc_req_id();
        self.forwarded.push(Forwarded { ours, source: req.source, theirs: req.req_id, line });
        ctx.scheduler.schedule(
            ctx.cycle + self.latency,
            downstream,
            ctx.self_id,
            Packet::Coh(CoherenceMsg::Req { txn: ours, line, kind, requester }),
        );
    }

    /// An upper copy's writeback while a probe for the line is being
    /// collected: its data belongs to the probe's answer, not to a line we
    /// are giving up.
    fn note_probe_writeback(&mut self, line: LineAddr, dirty: bool, ctx: &mut HandleCtx<'_>) {
        let Some(pending) = self.pending_probes.iter_mut().find(|p| p.line == line) else { return };
        pending.dirty |= dirty;
        pending.had_copy = true;
        if dirty && matches!(pending.origin, ProbeOrigin::Probe { .. }) {
            self.write_back(line, true, ctx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_response(
        &mut self,
        req_id: ReqId,
        line_addr: LineAddr,
        data: MemRespData,
        hit_level: HitLevel,
        granted: MesiState,
        ctx: &mut HandleCtx<'_>,
    ) {
        if self.writebacks.complete(req_id) {
            self.retry_blocked(ctx);
            return;
        }
        if let Some(index) = self.forwarded.iter().position(|f| f.ours == req_id) {
            let forwarded = self.forwarded.remove(index);
            ctx.scheduler.schedule(
                ctx.cycle,
                forwarded.source,
                ctx.self_id,
                Packet::MemResp {
                    req_id: forwarded.theirs,
                    line_addr,
                    data,
                    hit_level,
                    state: granted,
                },
            );
            return;
        }
        let Some(mshr) = self.mshrs.take(req_id) else { return };
        if self.full_mshr == Some(req_id) {
            self.full_mshr = None;
        }
        let installed = self.fill(&mshr, granted, ctx);
        if let Some(way) = self.find_way(mshr.line.val()) {
            let index = self.set_index(mshr.line.val()) * self.ways + way;
            for target in &mshr.targets {
                self.note_upper_copy(index, target.source);
            }
        }
        // The line is forwarded to its requests as it is written into the
        // array, without a second array access.
        let answered_at = ctx.cycle + self.response_latency;
        for target in &mshr.targets {
            let data = Self::serve(target.paddr, target.size, &target.op, ctx);
            self.respond(
                ctx,
                target.source,
                target.req_id,
                target.paddr,
                answered_at,
                hit_level,
                installed,
                data,
            );
        }
        self.serve_deferred(mshr.line, mshr.deferred, installed, hit_level, ctx);
        self.serve_after_fill(mshr.line, ctx);
        self.retry_blocked(ctx);
    }

    /// Serves the maintenance operations that waited for `line` to fill.
    fn serve_after_fill(&mut self, line: LineAddr, ctx: &mut HandleCtx<'_>) {
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.after_fill)
            .into_iter()
            .partition(|req| self.line_of(req.paddr.val()) == line);
        self.after_fill = waiting;
        for req in ready {
            self.on_request(req, ctx);
        }
    }

    /// Serves the targets a fill without write permission had to hold back:
    /// at once when the line arrived writable anyway, otherwise by fetching
    /// ownership of it first. The fill just freed an MSHR for that fetch.
    fn serve_deferred(
        &mut self,
        line: LineAddr,
        deferred: Vec<MshrTarget>,
        installed: MesiState,
        hit_level: HitLevel,
        ctx: &mut HandleCtx<'_>,
    ) {
        if deferred.is_empty() {
            return;
        }
        let writable = matches!(installed, MesiState::Exclusive | MesiState::Modified);
        let Some(way) = self.find_way(line.val()).filter(|_| writable) else {
            let vaddr = deferred.first().and_then(|t| t.vaddr);
            self.start_fetch(line, deferred, true, false, MemOp::ReadOwn, vaddr, ctx);
            return;
        };
        let index = self.set_index(line.val()) * self.ways + way;
        self.lines[index].state = MesiState::Modified;
        let answered_at = ctx.cycle + self.response_latency;
        for target in &deferred {
            self.note_upper_copy(index, target.source);
            let data = Self::serve(target.paddr, target.size, &target.op, ctx);
            self.respond(
                ctx,
                target.source,
                target.req_id,
                target.paddr,
                answered_at,
                hit_level,
                MesiState::Modified,
                data,
            );
        }
    }

    /// Installs a fetched line in the state the next level granted (a
    /// write installs it modified; a read never installs it dirtier than
    /// clean-exclusive), evicting a victim if the set is full. Returns the
    /// installed state.
    fn fill(&mut self, mshr: &Mshr, granted: MesiState, ctx: &mut HandleCtx<'_>) -> MesiState {
        let addr = mshr.line.val();
        let set_index = self.set_index(addr);
        let tag = self.tag_of(addr);
        ctx.stats.counter(self.stat_paths.fills).inc();

        let way = if let Some(way) = self.find_way(addr) {
            way
        } else if let Some(free) =
            (0..self.ways).find(|&w| !self.lines[set_index * self.ways + w].valid())
        {
            free
        } else {
            let victim = self.policy.get_victim(set_index);
            self.evict(set_index, victim, ctx);
            victim
        };
        let state = if mshr.write {
            MesiState::Modified
        } else {
            match granted {
                MesiState::Shared => MesiState::Shared,
                _ => MesiState::Exclusive,
            }
        };
        let index = set_index * self.ways + way;
        let state = if self.lines[index].valid() && self.lines[index].dirty() {
            MesiState::Modified
        } else {
            state
        };
        let upper = if self.lines[index].valid() { self.lines[index].upper } else { 0 };
        self.lines[index] = CacheLine { tag, state, upper };
        self.policy.update(set_index, way);
        state
    }

    /// Removes the line in `way` of `set_index`: dirty lines (and clean
    /// ones for an exclusive pair) go to the writeback buffer, and
    /// inclusive upper levels are told to drop their copies.
    fn evict(&mut self, set_index: usize, way: usize, ctx: &mut HandleCtx<'_>) {
        let index = set_index * self.ways + way;
        let victim = self.lines[index];
        if !victim.valid() {
            return;
        }
        ctx.stats.counter(self.stat_paths.evictions).inc();
        let line = self.line_of(self.reconstruct_addr(set_index, victim.tag));
        let holders = self.upper_holders(line);
        self.lines[index].state = MesiState::Invalid;
        if victim.dirty() || self.clean_victims_to_downstream {
            self.write_back(line, victim.dirty(), ctx);
        } else {
            self.notify_evict(line, ctx);
        }
        self.back_invalidate(line, &holders, ctx);
    }

    fn back_invalidate(&self, line: LineAddr, holders: &[ComponentId], ctx: &mut HandleCtx<'_>) {
        if self.upstream_inclusion != InclusionPolicy::Inclusive {
            return;
        }
        for &upstream in holders {
            ctx.scheduler.schedule(
                ctx.cycle,
                upstream,
                ctx.self_id,
                Packet::CacheInval { line_addr: line },
            );
        }
    }

    /// The next level dropped `line`; drop our copy too (writing it back
    /// first if it is dirty) and tell inclusive upper levels.
    fn on_back_invalidate(&mut self, line: LineAddr, ctx: &mut HandleCtx<'_>) {
        if !self.enabled {
            return;
        }
        let holders = self.upper_holders(line);
        if self.invalidate_line(line.val()) {
            self.write_back(line, true, ctx);
        }
        ctx.stats.counter(self.stat_paths.back_invalidations).inc();
        self.back_invalidate(line, &holders, ctx);
    }

    /// The bit `source` holds in a line's `upper` mask; zero when it is not
    /// a cache above this one.
    fn upstream_bit(&self, source: ComponentId) -> u8 {
        self.upstream.iter().position(|&up| up == source).map_or(0, |i| 1 << i)
    }

    /// `source` was given the line in `index`.
    fn note_upper_copy(&mut self, index: usize, source: ComponentId) {
        self.lines[index].upper |= self.upstream_bit(source);
    }

    /// `source` dropped its copy of the line holding `addr`.
    fn forget_upper_copy(&mut self, addr: u64, source: ComponentId) {
        if let Some(way) = self.find_way(addr) {
            let index = self.set_index(addr) * self.ways + way;
            self.lines[index].upper &= !self.upstream_bit(source);
        }
    }

    /// The caches above that may hold `line`: the ones given it while it is
    /// here; every one when it is not and they need not be inclusive (or
    /// this cache is disabled and holds nothing); none when they must be.
    fn upper_holders(&self, line: LineAddr) -> Vec<ComponentId> {
        if !self.enabled {
            return self.upstream.clone();
        }
        match self.find_way(line.val()) {
            Some(way) => {
                let bits = self.lines[self.set_index(line.val()) * self.ways + way].upper;
                self.upstream
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits & (1 << i) != 0)
                    .map(|(_, &up)| up)
                    .collect()
            }
            None if self.upstream_inclusion == InclusionPolicy::Inclusive => Vec::new(),
            None => self.upstream.clone(),
        }
    }

    /// A probe from the next level on behalf of a snoop: give up rights to
    /// the line here and in every cache above, then answer.
    fn on_probe(
        &mut self,
        line: LineAddr,
        kind: ProbeKind,
        txn: ReqId,
        from: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) {
        ctx.stats.counter(self.stat_paths.probes).inc();
        self.give_up_rights(line, kind, ProbeOrigin::Probe { from, txn }, true, ctx);
    }

    /// A snoop from the home agent: same as a probe, but the dirty data
    /// stays where it is (the snoop response tells the home it is the
    /// current copy) and the answer is a coherence message.
    fn on_snoop(&mut self, line: LineAddr, kind: SnoopKind, txn: ReqId, ctx: &mut HandleCtx<'_>) {
        ctx.stats.counter(self.stat_paths.snoops).inc();
        let probe_kind = match kind {
            SnoopKind::Shared => {
                ctx.stats.counter(self.stat_paths.snoop_downgrades).inc();
                ProbeKind::Downgrade
            }
            SnoopKind::Unique | SnoopKind::Invalid | SnoopKind::MakeInvalid => {
                ctx.stats.counter(self.stat_paths.snoop_invalidations).inc();
                ProbeKind::Invalidate
            }
            SnoopKind::Clean => ProbeKind::Clean,
        };
        self.give_up_rights(line, probe_kind, ProbeOrigin::Snoop { txn }, false, ctx);
    }

    fn give_up_rights(
        &mut self,
        line: LineAddr,
        kind: ProbeKind,
        origin: ProbeOrigin,
        write_back_dirty: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        // A fetch outstanding for the line was ordered after this probe at
        // the next level, so its fill is authoritative and is left alone.
        let had_copy = self.contains(line.val());
        let holders = self.upper_holders(line);
        let dirty =
            self.apply_probe(line, kind, write_back_dirty, ctx) || self.writebacks.holds(line);
        if holders.is_empty() {
            self.answer(origin, line, had_copy, dirty, ctx);
            return;
        }
        let ours = self.alloc_req_id();
        self.pending_probes.push(PendingProbe {
            ours,
            origin,
            line,
            kind,
            remaining: holders.len(),
            dirty,
            had_copy,
        });
        // Probes share the response path's delay so they cannot overtake a
        // response already sent to an upper cache.
        for &upstream in &holders {
            ctx.scheduler.schedule(
                ctx.cycle + self.latency,
                upstream,
                ctx.self_id,
                Packet::Probe { line_addr: line, kind, txn: ours },
            );
        }
    }

    /// Applies a probe to our own copy, writing a dirty line back first
    /// when asked. Returns whether the copy was dirty.
    fn apply_probe(
        &mut self,
        line: LineAddr,
        kind: ProbeKind,
        write_back_dirty: bool,
        ctx: &mut HandleCtx<'_>,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(way) = self.find_way(line.val()) else { return false };
        let index = self.set_index(line.val()) * self.ways + way;
        let dirty = self.lines[index].dirty();
        if dirty && write_back_dirty {
            self.write_back(line, true, ctx);
        }
        self.lines[index].state = match kind {
            ProbeKind::Invalidate => MesiState::Invalid,
            ProbeKind::Downgrade => MesiState::Shared,
            ProbeKind::Clean if dirty => MesiState::Exclusive,
            ProbeKind::Clean => self.lines[index].state,
        };
        dirty
    }

    fn answer(
        &self,
        origin: ProbeOrigin,
        line: LineAddr,
        had_copy: bool,
        dirty: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        // Never before the dirty data a probe writes back, which leaves a
        // cycle later.
        let at = ctx.cycle + self.latency.max(1);
        match origin {
            ProbeOrigin::Probe { from, txn } => ctx.scheduler.schedule(
                at,
                from,
                ctx.self_id,
                Packet::ProbeResp { line_addr: line, txn, had_copy, dirty },
            ),
            ProbeOrigin::Snoop { txn } => {
                let (Some(from), Some(downstream)) = (self.coherent, self.downstream) else {
                    return;
                };
                ctx.scheduler.schedule(
                    at,
                    downstream,
                    ctx.self_id,
                    Packet::Coh(CoherenceMsg::SnoopResp { txn, line, from, had_copy, dirty }),
                );
            }
        }
    }

    /// An upper cache answered one of our forwarded probes.
    fn on_probe_resp(
        &mut self,
        txn: ReqId,
        had_copy: bool,
        dirty: bool,
        from: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(index) = self.pending_probes.iter().position(|p| p.ours == txn) else { return };
        let (line, kind) = (self.pending_probes[index].line, self.pending_probes[index].kind);
        if !had_copy || kind == ProbeKind::Invalidate {
            self.forget_upper_copy(line.val(), from);
        }
        let pending = &mut self.pending_probes[index];
        pending.had_copy |= had_copy;
        pending.dirty |= dirty;
        pending.remaining -= 1;
        if pending.remaining > 0 {
            return;
        }
        let pending = self.pending_probes.remove(index);
        self.answer(pending.origin, pending.line, pending.had_copy, pending.dirty, ctx);
    }

    /// A coherence message from the home agent.
    fn on_coherence(&mut self, msg: CoherenceMsg, ctx: &mut HandleCtx<'_>) {
        match msg {
            CoherenceMsg::CompData { txn, line, state, .. }
            | CoherenceMsg::Comp { txn, line, state, .. } => {
                self.on_completion(txn, line, state, ctx);
            }
            CoherenceMsg::Snoop { txn, line, kind, .. } => self.on_snoop(line, kind, txn, ctx),
            CoherenceMsg::Req { .. }
            | CoherenceMsg::SnoopResp { .. }
            | CoherenceMsg::CompAck { .. }
            | CoherenceMsg::NoSnp { .. }
            | CoherenceMsg::NoSnpData { .. } => {}
        }
    }

    /// The home completed one of our requests: acknowledge it, then fill
    /// or retire the writeback. A permission grant for a line a snoop took
    /// away in the meantime is useless, so the fetch is re-issued for data.
    fn on_completion(
        &mut self,
        txn: ReqId,
        line: LineAddr,
        state: MesiState,
        ctx: &mut HandleCtx<'_>,
    ) {
        let (Some(from), Some(downstream)) = (self.coherent, self.downstream) else { return };
        ctx.scheduler.schedule(
            ctx.cycle,
            downstream,
            ctx.self_id,
            Packet::Coh(CoherenceMsg::CompAck { txn, line, from }),
        );
        let lost_upgrade =
            self.mshrs.iter().any(|m| m.req_id == txn && m.upgrade) && !self.contains(line.val());
        if lost_upgrade {
            self.reissue_for_data(txn, ctx);
            return;
        }
        self.on_response(txn, line, MemRespData::Small(0), HitLevel::L3, state, ctx);
    }

    fn reissue_for_data(&mut self, txn: ReqId, ctx: &mut HandleCtx<'_>) {
        let (Some(requester), Some(downstream)) = (self.coherent, self.downstream) else { return };
        let Some(mut mshr) = self.mshrs.take(txn) else { return };
        ctx.stats.counter(self.stat_paths.upgrade_retries).inc();
        mshr.req_id = self.alloc_req_id();
        mshr.upgrade = false;
        mshr.issued_at = ctx.cycle;
        let (req_id, line) = (mshr.req_id, mshr.line);
        self.mshrs.allocate(mshr);
        ctx.scheduler.schedule(
            ctx.cycle + self.latency,
            downstream,
            ctx.self_id,
            Packet::Coh(CoherenceMsg::Req {
                txn: req_id,
                line,
                kind: ReqKind::ReadUnique,
                requester,
            }),
        );
    }

    /// Re-presents queued requests while the cache has room for them.
    fn retry_blocked(&mut self, ctx: &mut HandleCtx<'_>) {
        while !self.is_blocked() {
            let Some(req) = self.blocked.pop_front() else { return };
            self.on_request(req, ctx);
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

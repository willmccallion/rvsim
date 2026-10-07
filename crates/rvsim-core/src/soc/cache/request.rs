//! Demand requests: lookup, responses, misses, fills and evictions.

use super::mshr::{Mshr, MshrTarget};
use crate::common::{LineAddr, PAGE_SHIFT, PhysAddr, VirtAddr};
use crate::config::InclusionPolicy;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::HandleCtx;
use crate::sim::packet::coherence::{CoherenceMsg, ReqKind};
use crate::sim::packet::{AccessSize, CacheLevel, HitLevel, MemOp, MemRespData, MesiState, Packet};

use super::Cache;
use super::{BlockedRequest, CacheLine};

impl Cache {
    pub(super) const fn alloc_req_id(&mut self) -> ReqId {
        let seq = self.next_req;
        self.next_req = seq.wrapping_add(1);
        ReqId::for_cache(self.id, seq)
    }

    pub(super) const fn is_blocked(&self) -> bool {
        self.mshrs.is_full() || self.full_mshr.is_some() || self.writebacks.is_full()
    }

    pub(super) const fn hit_level(&self) -> HitLevel {
        match self.level {
            CacheLevel::L1I | CacheLevel::L1D => HitLevel::L1,
            CacheLevel::L2 => HitLevel::L2,
            CacheLevel::L3 => HitLevel::L3,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn respond(
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
    pub(super) fn serve(
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

    pub(super) fn on_request(&mut self, req: BlockedRequest, ctx: &mut HandleCtx<'_>) {
        if let MemOp::Prefetch { into, exclusive } = req.op {
            self.on_prefetch_request(req.paddr, into, exclusive, ctx);
            return;
        }
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
        if let Some(way) = present {
            self.note_request_for(set_index * self.ways + way, ctx.stats);
        }
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
                self.drop_line(set_index * self.ways + way, ctx.stats);
                let _ = self.handed_up.insert(self.line_of(addr));
            }
            self.observe_prefetcher(addr, req.pc, true, ctx);
            return;
        }

        ctx.stats.counter(self.stat_paths.misses).inc();
        let line = self.line_of(addr);
        let target = MshrTarget {
            source: req.source,
            req_id: req.req_id,
            paddr: req.paddr,
            vaddr: req.vaddr,
            pc: req.pc,
            size: req.size,
            op: req.op,
        };
        if let Some(mshr) = self.mshrs.find_line_mut(line) {
            ctx.stats.counter(self.stat_paths.mshr_hits).inc();
            if mshr.prefetch && mshr.targets.is_empty() && mshr.deferred.is_empty() {
                ctx.stats.counter(self.stat_paths.prefetches_late).inc();
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
            self.start_fetch(line, vec![target], is_write, false, fetch_op, ctx);
            if is_write {
                self.observe_store_miss(addr, ctx);
            }
        }
        self.observe_prefetcher(addr, req.pc, false, ctx);
    }

    /// Allocates an MSHR for `line` and sends the line request downstream
    /// after the tag lookup, on behalf of its first target.
    pub(super) fn start_fetch(
        &mut self,
        line: LineAddr,
        targets: Vec<MshrTarget>,
        write: bool,
        prefetch: bool,
        op: MemOp,
        ctx: &mut HandleCtx<'_>,
    ) {
        let vaddr = targets.first().and_then(|target| target.vaddr);
        let pc = targets.first().and_then(|target| target.pc);
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
                Packet::MemReq { req_id, paddr: line.phys(), vaddr, pc, size: AccessSize::Line, op }
            }
        };
        ctx.scheduler.schedule(ctx.cycle + self.latency, downstream, ctx.self_id, packet);
    }

    /// Runs the prefetcher on a demand access and starts fetches for the
    /// lines it wants that are neither present nor already in flight,
    /// keeping one MSHR free for demand misses. A cache sees only physical
    /// addresses, and the page after this one may map anywhere, so like a
    /// hardware PA prefetcher it stays inside the smallest page.
    pub(super) fn observe_prefetcher(
        &mut self,
        addr: u64,
        pc: Option<VirtAddr>,
        hit: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(prefetcher) = self.prefetcher.as_mut() else { return };
        let candidates = prefetcher.observe(addr, pc, hit);
        for candidate in candidates {
            if !same_base_page(candidate, addr) {
                ctx.stats.counter(self.stat_paths.prefetches_page_crossing).inc();
                continue;
            }
            if self.mshrs.free() <= 1 || self.downstream.is_none() {
                return;
            }
            self.start_prefetch(candidate, false, ctx);
        }
    }

    /// Trains the store prefetcher on a store miss that started a fetch for
    /// write permission, and sends the lines it wants to the next level.
    fn observe_store_miss(&mut self, addr: u64, ctx: &mut HandleCtx<'_>) {
        let Some(prefetcher) = self.store_prefetcher.as_mut() else { return };
        let Some(into) = next_level(self.level) else { return };
        for line in prefetcher.observe(addr) {
            ctx.stats.counter(self.stat_paths.store_prefetches).inc();
            self.pass_prefetch_down(PhysAddr::new(line), into, true, ctx);
        }
    }

    /// A prefetch request from above: passed down toward the level it
    /// fills, and started there. One for a disabled level, or that has no
    /// cache below to go to, is dropped.
    fn on_prefetch_request(
        &mut self,
        paddr: PhysAddr,
        into: CacheLevel,
        exclusive: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        if into != self.level {
            self.pass_prefetch_down(paddr, into, exclusive, ctx);
            return;
        }
        if !self.enabled {
            return;
        }
        if self.is_blocked() || self.mshrs.free() <= 1 || self.downstream.is_none() {
            ctx.stats.counter(self.stat_paths.prefetches_dropped).inc();
            return;
        }
        self.start_prefetch(paddr.val(), exclusive, ctx);
    }

    /// Sends a prefetch for a lower level on to the cache below.
    fn pass_prefetch_down(
        &self,
        paddr: PhysAddr,
        into: CacheLevel,
        exclusive: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(downstream @ ComponentId::Cache(_)) = self.downstream else { return };
        let packet = Packet::MemReq {
            req_id: ReqId::new(0),
            paddr,
            vaddr: None,
            pc: None,
            size: AccessSize::Line,
            op: MemOp::Prefetch { into, exclusive },
        };
        ctx.scheduler.schedule(ctx.cycle, downstream, ctx.self_id, packet);
    }

    /// Starts fetching the line holding `addr` as a prefetch, unless it is
    /// already here with the permission wanted, in flight or being
    /// written back.
    fn start_prefetch(&mut self, addr: u64, exclusive: bool, ctx: &mut HandleCtx<'_>) {
        let line = self.line_of(addr);
        if self.holds_for(addr, exclusive)
            || self.mshrs.holds(line)
            || self.writebacks.holds(line)
            || self.handed_up.contains(&line)
        {
            return;
        }
        ctx.stats.counter(self.stat_paths.prefetches_issued).inc();
        let op = if exclusive { MemOp::ReadOwn } else { MemOp::Read };
        self.start_fetch(line, Vec::new(), exclusive, true, op, ctx);
    }

    /// True when the line holding `addr` is here, writable if `exclusive`.
    fn holds_for(&self, addr: u64, exclusive: bool) -> bool {
        let Some(way) = self.find_way(addr) else { return false };
        let state = self.lines[self.set_index(addr) * self.ways + way].state;
        !exclusive || state != MesiState::Shared
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn on_response(
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
        let installed = if self.hands_line_up(&mshr) {
            let _ = self.handed_up.insert(mshr.line);
            fetched_state(&mshr, granted)
        } else {
            self.fill(&mshr, granted, ctx)
        };
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

    /// The way `addr`'s line goes in: its own when held, else a free one,
    /// else the replacement policy's victim, evicted to make room.
    fn way_for(&mut self, addr: u64, ctx: &mut HandleCtx<'_>) -> usize {
        let set_index = self.set_index(addr);
        if let Some(way) = self.find_way(addr) {
            return way;
        }
        if let Some(free) = (0..self.ways).find(|&w| !self.lines[set_index * self.ways + w].valid())
        {
            return free;
        }
        let victim = self.policy.get_victim(set_index);
        self.evict(set_index, victim, ctx);
        victim
    }

    /// Installs a line a cache above evicted, as an exclusive level holds
    /// what the level above gives up; it is modified when `dirty`.
    pub(super) fn install_victim(&mut self, line: LineAddr, dirty: bool, ctx: &mut HandleCtx<'_>) {
        let addr = line.val();
        let set_index = self.set_index(addr);
        let way = self.way_for(addr, ctx);
        let state = if dirty { MesiState::Modified } else { MesiState::Exclusive };
        let tag = self.tag_of(addr);
        self.lines[set_index * self.ways + way] =
            CacheLine { tag, state, upper: 0, prefetched: false };
        self.policy.update(set_index, way);
    }

    /// True when this cache, exclusive of the caches above it, fetched the
    /// line only for them: it passes the line up without keeping a copy,
    /// as an exclusive level holds only what the level above evicts.
    fn hands_line_up(&self, mshr: &Mshr) -> bool {
        self.upstream_inclusion == InclusionPolicy::Exclusive
            && !mshr.targets.is_empty()
            && mshr
                .targets
                .iter()
                .chain(&mshr.deferred)
                .all(|t| matches!(t.source, ComponentId::Cache(_)))
    }

    /// Serves the maintenance operations that waited for `line` to fill.
    pub(super) fn serve_after_fill(&mut self, line: LineAddr, ctx: &mut HandleCtx<'_>) {
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
    pub(super) fn serve_deferred(
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
            self.start_fetch(line, deferred, true, false, MemOp::ReadOwn, ctx);
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
    pub(super) fn fill(
        &mut self,
        mshr: &Mshr,
        granted: MesiState,
        ctx: &mut HandleCtx<'_>,
    ) -> MesiState {
        let addr = mshr.line.val();
        let set_index = self.set_index(addr);
        let tag = self.tag_of(addr);
        ctx.stats.counter(self.stat_paths.fills).inc();

        let way = self.way_for(addr, ctx);
        let state = fetched_state(mshr, granted);
        let index = set_index * self.ways + way;
        let state = if self.lines[index].valid() && self.lines[index].dirty() {
            MesiState::Modified
        } else {
            state
        };
        let kept = self.lines[index].valid().then_some(self.lines[index]);
        let upper = kept.map_or(0, |line| line.upper);
        let unrequested_prefetch =
            mshr.prefetch && mshr.targets.is_empty() && mshr.deferred.is_empty();
        let prefetched = unrequested_prefetch || kept.is_some_and(|line| line.prefetched);
        self.lines[index] = CacheLine { tag, state, upper, prefetched };
        self.policy.update(set_index, way);
        state
    }

    /// Removes the line in `way` of `set_index`: dirty lines (and clean
    /// ones for an exclusive pair) go to the writeback buffer, and
    /// inclusive upper levels are told to drop their copies.
    pub(super) fn evict(&mut self, set_index: usize, way: usize, ctx: &mut HandleCtx<'_>) {
        let index = set_index * self.ways + way;
        let victim = self.lines[index];
        if !victim.valid() {
            return;
        }
        ctx.stats.counter(self.stat_paths.evictions).inc();
        let line = self.line_of(self.reconstruct_addr(set_index, victim.tag));
        let holders = self.upper_holders(line);
        self.drop_line(index, ctx.stats);
        if victim.dirty() || self.clean_victims_to_downstream {
            self.write_back(line, victim.dirty(), ctx);
        } else {
            self.notify_evict(line, ctx);
        }
        self.back_invalidate(line, &holders, ctx);
    }

    /// Re-presents queued requests while the cache has room for them.
    pub(super) fn retry_blocked(&mut self, ctx: &mut HandleCtx<'_>) {
        while !self.is_blocked() {
            let Some(req) = self.blocked.pop_front() else { return };
            self.on_request(req, ctx);
        }
    }
}

/// The state a fetched line arrives in: modified for a write, otherwise as
/// the next level granted it, never dirtier than clean-exclusive.
const fn fetched_state(mshr: &Mshr, granted: MesiState) -> MesiState {
    if mshr.write {
        return MesiState::Modified;
    }
    match granted {
        MesiState::Shared => MesiState::Shared,
        _ => MesiState::Exclusive,
    }
}

/// True when `a` and `b` lie in the same 4 KiB page, the smallest a
/// translation can map.
const fn same_base_page(a: u64, b: u64) -> bool {
    a >> PAGE_SHIFT == b >> PAGE_SHIFT
}

/// The cache level below `level`, if there is one.
const fn next_level(level: CacheLevel) -> Option<CacheLevel> {
    match level {
        CacheLevel::L1I | CacheLevel::L1D => Some(CacheLevel::L2),
        CacheLevel::L2 => Some(CacheLevel::L3),
        CacheLevel::L3 => None,
    }
}

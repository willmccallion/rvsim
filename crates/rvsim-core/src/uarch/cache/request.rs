//! Demand requests: lookup, responses, misses, fills and evictions.

use super::mshr::{Mshr, MshrTarget};
use crate::common::{LineAddr, PhysAddr, VirtAddr};
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
    pub(super) fn start_fetch(
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
    pub(super) fn observe_prefetcher(&mut self, addr: u64, hit: bool, ctx: &mut HandleCtx<'_>) {
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
    pub(super) fn evict(&mut self, set_index: usize, way: usize, ctx: &mut HandleCtx<'_>) {
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

    /// Re-presents queued requests while the cache has room for them.
    pub(super) fn retry_blocked(&mut self, ctx: &mut HandleCtx<'_>) {
        while !self.is_blocked() {
            let Some(req) = self.blocked.pop_front() else { return };
            self.on_request(req, ctx);
        }
    }
}

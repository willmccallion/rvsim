//! The requester side of the coherence protocol: requests to the home,
//! snoops, probes of the upper levels, and completions.

use crate::common::LineAddr;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::HandleCtx;
use crate::sim::packet::coherence::{CoherenceMsg, ReqKind, SnoopKind};
use crate::sim::packet::{HitLevel, MemOp, MemRespData, MesiState, Packet, ProbeKind};

use super::Cache;
use super::writeback_buffer::WritebackCause;
use super::{AfterProbe, BlockedRequest, Forwarded, PendingProbe, ProbeOrigin};

impl Cache {
    /// A disabled cache that is still its core's requesting agent: the
    /// caches above it hold the lines, so their line requests, writebacks
    /// and evictions are spoken to the home on their behalf.
    pub(super) fn forward_as_coherence_request(
        &mut self,
        req: &BlockedRequest,
        ctx: &mut HandleCtx<'_>,
    ) {
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
                    self.write_back(line, true, WritebackCause::Eviction, ctx);
                } else {
                    self.notify_evict(line, ctx);
                }
                return;
            }
            MemOp::Write { .. }
            | MemOp::ReadOwn
            | MemOp::Atomic { .. }
            | MemOp::Prefetch { exclusive: true, .. } => ReqKind::ReadUnique,
            MemOp::Read | MemOp::Fetch | MemOp::Prefetch { exclusive: false, .. } => {
                ReqKind::ReadShared
            }
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

    /// True while a probe or snoop for `line` is collecting the answers of
    /// the caches above, so their writebacks of it belong to its answer.
    pub(super) fn answering_a_probe_for(&self, line: LineAddr) -> bool {
        self.pending_probes
            .iter()
            .any(|p| p.line == line && matches!(p.then, AfterProbe::Answer(_)))
    }

    /// An upper copy's writeback while a probe for the line is being
    /// collected: its data belongs to the probe's answer, not to a line we
    /// are giving up.
    pub(super) fn note_probe_writeback(
        &mut self,
        line: LineAddr,
        dirty: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(pending) = self
            .pending_probes
            .iter_mut()
            .find(|p| p.line == line && matches!(p.then, AfterProbe::Answer(_)))
        else {
            return;
        };
        pending.dirty |= dirty;
        pending.had_copy = true;
        if dirty && matches!(pending.then, AfterProbe::Answer(ProbeOrigin::Probe { .. })) {
            self.write_back(line, true, WritebackCause::Demanded, ctx);
        }
    }

    /// A probe from the next level on behalf of a snoop: give up rights to
    /// the line here and in every cache above, then answer.
    pub(super) fn on_probe(
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
    pub(super) fn on_snoop(
        &mut self,
        line: LineAddr,
        kind: SnoopKind,
        txn: ReqId,
        ctx: &mut HandleCtx<'_>,
    ) {
        ctx.stats.counter(self.stat_paths.snoops).inc();
        let probe_kind = match kind {
            SnoopKind::Shared => ProbeKind::Downgrade,
            SnoopKind::Unique | SnoopKind::Invalid | SnoopKind::MakeInvalid => {
                ProbeKind::Invalidate
            }
            SnoopKind::Clean => ProbeKind::Clean,
        };
        self.give_up_rights(line, probe_kind, ProbeOrigin::Snoop { txn }, false, ctx);
    }

    pub(super) fn give_up_rights(
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
            self.answer(origin, line, kind, had_copy, dirty, ctx);
            return;
        }
        let ours = self.alloc_req_id();
        self.pending_probes.push(PendingProbe {
            ours,
            then: AfterProbe::Answer(origin),
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
    pub(super) fn apply_probe(
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
            self.write_back(line, true, WritebackCause::Demanded, ctx);
        }
        match kind {
            ProbeKind::Invalidate => self.drop_line(index, ctx.stats),
            ProbeKind::Downgrade => self.lines[index].state = MesiState::Shared,
            ProbeKind::Clean if dirty => self.lines[index].state = MesiState::Exclusive,
            ProbeKind::Clean => {}
        }
        dirty
    }

    /// Answers a probe or snoop for `line` once every copy here and above
    /// has given up what `kind` asked; a snoop that found a copy counts as
    /// an invalidation or a downgrade.
    pub(super) fn answer(
        &self,
        origin: ProbeOrigin,
        line: LineAddr,
        kind: ProbeKind,
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
                Packet::ProbeResp { txn, had_copy, dirty },
            ),
            ProbeOrigin::Snoop { txn } => {
                let lost = match kind {
                    ProbeKind::Invalidate => Some(self.stat_paths.snoop_invalidations),
                    ProbeKind::Downgrade => Some(self.stat_paths.snoop_downgrades),
                    ProbeKind::Clean => None,
                };
                if let Some(stat) = lost.filter(|_| had_copy) {
                    ctx.stats.counter(stat).inc();
                }
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
    pub(super) fn on_probe_resp(
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
        let PendingProbe { then, line, kind, had_copy, dirty, .. } = pending;
        match then {
            AfterProbe::Answer(origin) => self.answer(origin, line, kind, had_copy, dirty, ctx),
            AfterProbe::ServeFetch => self.serve_probed_fetch(txn, dirty, ctx),
        }
    }

    /// A coherence message from the home agent.
    pub(super) fn on_coherence(&mut self, msg: CoherenceMsg, ctx: &mut HandleCtx<'_>) {
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
    pub(super) fn on_completion(
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

    pub(super) fn reissue_for_data(&mut self, txn: ReqId, ctx: &mut HandleCtx<'_>) {
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
}

//! Keeping instruction fetches coherent with the data cache, as FENCE.I
//! requires. Where the L1I and L1D share a cache below them, that cache
//! probes the L1D for a writable copy of a line an instruction fetch
//! wants, as the coherent L2s of Rocket and the U74 do. Where they share
//! none, FENCE.I flushes the L1D instead, as Rocket does when nothing
//! tracks its cached executable memory: the cache walks every line, one a
//! cycle, writes back the dirty ones and invalidates them all, skipping
//! the walk when nothing has been fetched since the last flush.

use crate::common::LineAddr;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::HandleCtx;
use crate::sim::packet::{MemOp, MemRespData, MesiState, Packet, ProbeKind};

use super::writeback_buffer::WritebackCause;
use super::{AfterProbe, BlockedRequest, Cache, FlushWalk, PendingProbe};

impl Cache {
    /// Before serving an instruction fetch, probes the caches above that
    /// may hold its line writable, so a dirty copy reaches this cache
    /// first. Returns the fetch when nothing needs probing; otherwise holds
    /// it until the probes are answered.
    pub(super) fn probe_before_fetch(
        &mut self,
        req: BlockedRequest,
        ctx: &mut HandleCtx<'_>,
    ) -> Option<BlockedRequest> {
        if req.probed_above || !matches!(req.op, MemOp::Fetch) {
            return Some(req);
        }
        let line = self.line_of(req.paddr.val());
        let holders = self.writable_upper_copies(line, req.source);
        if holders.is_empty() {
            return Some(req);
        }
        ctx.stats.counter(self.stat_paths.fetch_probes).inc();
        let ours = self.alloc_req_id();
        self.pending_probes.push(PendingProbe {
            ours,
            then: AfterProbe::ServeFetch,
            line,
            kind: ProbeKind::Clean,
            remaining: holders.len(),
            dirty: false,
            had_copy: false,
        });
        self.fetches_awaiting_probes.push((ours, req));
        for upstream in holders {
            ctx.scheduler.schedule(
                ctx.cycle + self.latency,
                upstream,
                ctx.self_id,
                Packet::Probe { line_addr: line, kind: ProbeKind::Clean, txn: ours },
            );
        }
        None
    }

    /// The caches above, other than `requester`, that may hold `line` with
    /// write permission. While this cache holds the line, those it was
    /// given to, unless it holds the line Shared and so granted it Shared.
    /// An exclusive cache that handed the line up knows a cache above has
    /// it. A non-inclusive cache that has evicted the line keeps no record
    /// of the caches above that still hold it, so it probes none.
    fn writable_upper_copies(&self, line: LineAddr, requester: ComponentId) -> Vec<ComponentId> {
        let holders = match self.find_way(line.val()) {
            Some(way) => {
                let held = self.lines[self.set_index(line.val()) * self.ways + way];
                if held.state == MesiState::Shared {
                    return Vec::new();
                }
                self.upper_holders(line)
            }
            None if self.handed_up.contains(&line) => self.upstream.clone(),
            None => return Vec::new(),
        };
        holders.into_iter().filter(|&holder| holder != requester).collect()
    }

    /// A cache above wrote `line` back while an instruction fetch's probe
    /// for it was outstanding: the probe found a dirty copy.
    pub(super) fn note_fetch_probe_writeback(&mut self, line: LineAddr, dirty: bool) {
        let fetch_probe =
            |p: &&mut PendingProbe| p.line == line && matches!(p.then, AfterProbe::ServeFetch);
        if let Some(pending) = self.pending_probes.iter_mut().find(fetch_probe) {
            pending.dirty |= dirty;
        }
    }

    /// Every cache above answered the probes for the fetch held under
    /// `ours`: serve it.
    pub(super) fn serve_probed_fetch(&mut self, ours: ReqId, dirty: bool, ctx: &mut HandleCtx<'_>) {
        let Some(index) = self.fetches_awaiting_probes.iter().position(|(id, _)| *id == ours)
        else {
            return;
        };
        let (_, mut req) = self.fetches_awaiting_probes.remove(index);
        if dirty {
            ctx.stats.counter(self.stat_paths.fetch_probes_dirty).inc();
        }
        req.probed_above = true;
        self.on_request(req, ctx);
    }

    /// Starts flushing every line for `requester`, which is answered once
    /// the last writeback is acknowledged: at once when nothing has been
    /// fetched since the last flush, and with the flush under way when one
    /// already is.
    pub(super) fn on_flush_all(
        &mut self,
        requester: ComponentId,
        req_id: ReqId,
        ctx: &mut HandleCtx<'_>,
    ) {
        if let Some(walk) = self.flush.as_mut() {
            walk.requesters.push((requester, req_id));
            return;
        }
        if !self.filled_since_flush {
            self.answer_flush(requester, req_id, ctx);
            return;
        }
        ctx.stats.counter(self.stat_paths.flushes).inc();
        self.filled_since_flush = false;
        self.flush = Some(FlushWalk { requesters: vec![(requester, req_id)], next_line: 0 });
        self.on_flush_step(ctx);
    }

    /// Flushes the walk's next line: writes it back first when it is dirty,
    /// waiting a cycle while the writeback buffer is full, then invalidates
    /// it. A clean line is dropped without a message, as nothing below
    /// records which lines this cache holds.
    pub(super) fn on_flush_step(&mut self, ctx: &mut HandleCtx<'_>) {
        let Some(index) = self.flush.as_ref().map(|walk| walk.next_line) else { return };
        if index == self.lines.len() {
            self.finish_flush(ctx);
            return;
        }
        let line = self.lines[index];
        if line.valid() {
            if line.dirty() && self.writebacks.is_full() {
                Self::schedule_flush_step(ctx);
                return;
            }
            let addr = self.reconstruct_addr(index / self.ways, line.tag);
            if line.dirty() {
                self.write_back(self.line_of(addr), true, WritebackCause::Flushed, ctx);
            }
            self.drop_line(index, ctx.stats);
            ctx.stats.counter(self.stat_paths.flushed_lines).inc();
        }
        if let Some(walk) = self.flush.as_mut() {
            walk.next_line += 1;
        }
        Self::schedule_flush_step(ctx);
    }

    fn schedule_flush_step(ctx: &mut HandleCtx<'_>) {
        ctx.scheduler.schedule(ctx.cycle + 1, ctx.self_id, ctx.self_id, Packet::FlushStep);
    }

    /// Answers a flush whose walk is over once its writebacks are
    /// acknowledged, and lets the requests it held in.
    pub(super) fn finish_flush(&mut self, ctx: &mut HandleCtx<'_>) {
        let walked = self.flush.as_ref().is_some_and(|walk| walk.next_line == self.lines.len());
        if !walked || self.writebacks.holds_cause(WritebackCause::Flushed) {
            return;
        }
        let Some(walk) = self.flush.take() else { return };
        for (requester, req_id) in walk.requesters {
            self.answer_flush(requester, req_id, ctx);
        }
        self.retry_blocked(ctx);
    }

    /// Tells `requester` its flush is done.
    fn answer_flush(&self, requester: ComponentId, req_id: ReqId, ctx: &mut HandleCtx<'_>) {
        ctx.scheduler.schedule(
            ctx.cycle + self.latency,
            requester,
            ctx.self_id,
            Packet::MemResp {
                req_id,
                line_addr: self.line_of(0),
                data: MemRespData::Small(0),
                hit_level: self.hit_level(),
                state: MesiState::Invalid,
            },
        );
    }
}

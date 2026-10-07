//! Writebacks to the next level and cache-maintenance operations.

use super::writeback_buffer::{Writeback, WritebackCause};
use crate::common::LineAddr;
use crate::sim::components::ComponentId;
use crate::sim::handle::HandleCtx;
use crate::sim::packet::coherence::{CoherenceMsg, ReqKind};
use crate::sim::packet::{AccessSize, Maintenance, MemOp, MemRespData, MesiState, Packet};

use super::Cache;
use super::{BlockedRequest, Forwarded};

impl Cache {
    /// A whole line arriving from above: merge into our copy when we hold
    /// it, otherwise pass it on without allocating. The requester is
    /// acknowledged after the access latency either way.
    pub(super) fn on_writeback(
        &mut self,
        req: &BlockedRequest,
        dirty: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
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
            // A writeback carries data, never permission. Held Shared, the
            // line was downgraded by a probe whose answer already carried
            // this dirty data; the writeback merely arrived after it.
            if dirty && self.lines[index].state != MesiState::Shared {
                self.lines[index].state = MesiState::Modified;
            }
            return;
        }
        if self.handed_up.remove(&line) {
            self.install_victim(line, dirty, ctx);
            return;
        }
        if dirty || self.clean_victims_to_downstream {
            // Not ours: forward downstream through the writeback buffer.
            self.write_back(self.line_of(addr), dirty, WritebackCause::Eviction, ctx);
        }
    }

    /// Sends a line to the next level and tracks it until acknowledged.
    pub(super) fn write_back(
        &mut self,
        line: LineAddr,
        dirty: bool,
        cause: WritebackCause,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(downstream) = self.downstream else { return };
        let req_id = self.alloc_req_id();
        self.writebacks.allocate(Writeback { line, req_id, cause });
        ctx.stats.counter(self.stat_paths.writebacks).inc();
        let packet = self.coherent.map_or_else(
            || Packet::MemReq {
                req_id,
                paddr: line.phys(),
                vaddr: None,
                pc: None,
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
    pub(super) fn notify_evict(&mut self, line: LineAddr, ctx: &mut HandleCtx<'_>) {
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
    pub(super) fn forward(&mut self, req: BlockedRequest, ctx: &mut HandleCtx<'_>) {
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
                pc: req.pc,
                size: req.size,
                op: req.op,
            },
        );
    }

    /// A maintenance operation from above: applied to our copy once any
    /// fetch of the line has filled, then passed on toward memory with our
    /// dirty data, as gem5's cache always forwards one.
    pub(super) fn on_maintain(
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
                pc: None,
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
    pub(super) fn apply_maintenance(
        &mut self,
        line: LineAddr,
        op: Maintenance,
        requester: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) -> bool {
        match op {
            Maintenance::Clean => self.clean_line(line.val()),
            Maintenance::Flush | Maintenance::Invalidate => {
                let _ = self.handed_up.remove(&line);
                let holders: Vec<ComponentId> =
                    self.upper_holders(line).into_iter().filter(|&h| h != requester).collect();
                self.back_invalidate(line, &holders, ctx);
                self.invalidate_line(line.val(), ctx.stats)
            }
        }
    }
}

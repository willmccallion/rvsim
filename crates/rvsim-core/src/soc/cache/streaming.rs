//! Write streaming: whole-line writes in a run skip the caches that would
//! otherwise read each line before overwriting it (see `write_stream`).

use super::request::Response;
use super::write_stream::{StreamPlacement, WriteStreamDetector};
use super::writeback_buffer::{Writeback, WritebackCause};
use super::{BlockedRequest, Cache};
use crate::common::LineAddr;
use crate::config::WriteStreamingConfig;
use crate::sim::handle::HandleCtx;
use crate::sim::packet::coherence::{CoherenceMsg, ReqKind};
use crate::sim::packet::{
    AccessSize, MemOp, MemRespData, MesiState, Packet, WriteData, WriteOrigin,
};

impl Cache {
    /// Lets runs of whole-line hart writes stop allocating here and below.
    pub const fn set_write_streaming(&mut self, config: WriteStreamingConfig) {
        self.write_stream = Some(WriteStreamDetector::new(config, self.line_bytes as u64));
    }

    /// Where a hart's write would go if it missed: counted by the run
    /// detector when there is one, otherwise always this cache.
    pub(super) fn stream_placement(&mut self, req: &BlockedRequest) -> StreamPlacement {
        let MemOp::Write { data, origin: WriteOrigin::Hart(_) } = &req.op else {
            return StreamPlacement::L1;
        };
        let whole = matches!(data, WriteData::Line { mask, .. } if *mask == self.whole_line_mask())
            && req.size == AccessSize::Line;
        let line = self.line_of(req.paddr);
        let Some(detector) = self.write_stream.as_mut() else { return StreamPlacement::L1 };
        if whole {
            detector.whole_line(line)
        } else {
            detector.partial_line();
            StreamPlacement::L1
        }
    }

    /// The write mask that covers every byte of a line.
    const fn whole_line_mask(&self) -> u64 {
        if self.line_bytes >= 64 { u64::MAX } else { (1 << self.line_bytes) - 1 }
    }

    /// Performs a streamed whole-line write that missed, acknowledges it,
    /// and sends the line down without fetching or keeping it.
    pub(super) fn stream_write(
        &mut self,
        req: &BlockedRequest,
        placement: StreamPlacement,
        ctx: &mut HandleCtx<'_>,
    ) {
        let data = Self::serve(req.paddr, req.size, &req.op, ctx);
        self.acknowledge(req, data, ctx);
        self.send_streamed(self.line_of(req.paddr), placement == StreamPlacement::L2, ctx);
    }

    /// A streamed line from the cache above, already in RAM: merged into
    /// our copy, kept without a fetch when `allocate` asks for it, or
    /// passed on. A line we are fetching, or hold only to read, waits for
    /// write permission like any write.
    pub(super) fn on_write_full(
        &mut self,
        req: BlockedRequest,
        allocate: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let line = self.line_of(req.paddr);
        let present = self.find_way(line);
        let held_shared = present.is_some_and(|way| {
            self.lines[self.slot(self.set_index(line), way)].state == MesiState::Shared
        });
        // A coherent cache may not keep a line the home has not granted it,
        // so it takes ownership with a fetch (CHI would send MakeUnique).
        let needs_ownership = allocate && present.is_none() && self.coherent.is_some();
        if self.mshrs.holds(line) || held_shared || needs_ownership {
            self.join_or_start_fetch(req, true, ctx);
            return;
        }
        self.acknowledge(&req, MemRespData::Small(0), ctx);
        match present {
            Some(way) => {
                let set_index = self.set_index(line);
                self.policy.update(set_index, way);
                let index = self.slot(set_index, way);
                self.lines[index].state = MesiState::Modified;
            }
            None if allocate => {
                ctx.stats.counter(self.stat_paths.streamed_installs).inc();
                self.install_victim(line, true, ctx);
            }
            None => self.send_streamed(line, false, ctx),
        }
    }

    /// Answers `req` after the access latency, holding no copy for it.
    fn acknowledge(&self, req: &BlockedRequest, data: MemRespData, ctx: &mut HandleCtx<'_>) {
        let response = Response {
            target: req.source,
            req_id: req.req_id,
            paddr: req.paddr,
            at: ctx.cycle + self.latency,
            hit_level: self.hit_level(),
            state: MesiState::Invalid,
            data,
        };
        self.respond(response, ctx);
    }

    /// Sends a streamed line down through the writeback buffer: to the
    /// next cache with whether to keep it, or, from a coherent cache, to
    /// the home as a `WriteUniqueFull`, which invalidates every copy.
    pub(super) fn send_streamed(
        &mut self,
        line: LineAddr,
        allocate: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(downstream) = self.downstream else { return };
        let req_id = self.alloc_req_id();
        self.writebacks.allocate(Writeback { line, req_id, cause: WritebackCause::Streamed });
        ctx.stats.counter(self.stat_paths.streamed_writes).inc();
        let packet = self.coherent.map_or_else(
            || Packet::MemReq {
                req_id,
                paddr: line.phys(),
                vaddr: None,
                pc: None,
                size: AccessSize::Line,
                op: MemOp::WriteFull { allocate },
            },
            |requester| {
                Packet::Coh(CoherenceMsg::Req {
                    txn: req_id,
                    line,
                    kind: ReqKind::WriteUniqueFull,
                    requester,
                })
            },
        );
        ctx.scheduler.schedule(ctx.cycle + 1, downstream, ctx.self_id, packet);
    }
}

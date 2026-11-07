//! Rename for the in-order engine: a ROB entry per instruction and
//! scoreboard tags naming the producer of each source operand.
//!
//! Source tags are captured before the destination is claimed, so an
//! instruction reading its own destination (`addi x5, x5, 16`) sees the
//! previous producer.

use super::InOrderEngine;
use super::issue::ProducerTags;
use crate::exec::compute::vector::mem::is_vec_store;
use crate::exec::inst::Inst;
use crate::trace_rename;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::engine::{ExecutionEngine, Renamed};
use crate::uarch::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::trace::Stage;

impl InOrderEngine {
    /// Renames one decoded instruction and allocates its ROB and store
    /// buffer slots. A store waits here, before anything is allocated, when
    /// its buffer is full; `can_accept` covers the ROB and issue-queue slots
    /// every instruction needs.
    pub(super) fn rename_one(
        &mut self,
        state: &mut StageCtx<'_>,
        decoded: IdExEntry,
    ) -> Renamed<ProducerTags> {
        let store_slot = if decoded.inst.ctrl.uses_store_buffer() {
            !self.store_buffer.is_full()
        } else if is_vec_store(decoded.inst.ctrl.vec_op) {
            self.vec_store_buffer.free_slots() > 0
        } else {
            true
        };
        if !store_slot {
            return Renamed::Stalled(Box::new(decoded));
        }
        let vector = self.vector_config(&state.hart().csrs);
        let Some(rob_tag) = self.rob.allocate(
            decoded.inst.pc,
            decoded.inst.bits,
            decoded.inst.size,
            decoded.inst.rd,
            decoded.inst.ctrl,
            PhysReg(0),
            PhysReg(0),
            decoded.seq,
        ) else {
            return Renamed::Stalled(Box::new(decoded));
        };
        state.trace_stage(decoded.seq, Stage::Rename);

        // Capture source tags BEFORE updating scoreboard for rd.
        let ctrl = decoded.inst.ctrl;
        let rs1_tag = self.scoreboard.get_producer(ctrl.src1(decoded.inst.rs1));
        let rs2_tag = self.scoreboard.get_producer(ctrl.src2(decoded.inst.rs2));
        let rs3_tag = ctrl.src3(decoded.inst.rs3).and_then(|rs3| self.scoreboard.get_producer(rs3));

        if let Some(dest) = ctrl.dest(decoded.inst.rd) {
            self.scoreboard.set_producer(dest, rob_tag);
        }

        let slot_allocated = if decoded.inst.ctrl.uses_store_buffer() {
            self.store_buffer.allocate(rob_tag, decoded.inst.ctrl.width)
        } else if is_vec_store(decoded.inst.ctrl.vec_op) {
            self.vec_store_buffer.allocate(rob_tag)
        } else {
            true
        };
        debug_assert!(slot_allocated, "the store slot was checked before allocating");

        let entry = RenameIssueEntry {
            inst: Inst { rv1: 0, rv2: 0, rv3: 0, ..decoded.inst },
            rob_tag,
            trap: decoded.trap,
            predicted_target: decoded.predicted_target,
            seq: decoded.seq,
            vector_config: vector,
            vector_rounding: state.hart().csrs.vector_rounding(),
            regs: ProducerTags { rs1: rs1_tag, rs2: rs2_tag, rs3: rs3_tag },
        };

        trace_rename!(state.config.general.trace_instructions;
            pc         = %crate::common::trace::Hex(entry.inst.pc.val()),
            rob_tag    = %entry.rob_tag,
            rd         = entry.inst.rd.as_usize(),
            rs1        = entry.inst.rs1.as_usize(),
            rs1_tag    = ?entry.regs.rs1,
            rs2        = entry.inst.rs2.as_usize(),
            rs2_tag    = ?entry.regs.rs2,
            is_store   = entry.inst.ctrl.mem_write,
            is_load    = entry.inst.ctrl.mem_read,
            "RN: in-order rename"
        );

        Renamed::Accepted(Box::new(entry))
    }
}

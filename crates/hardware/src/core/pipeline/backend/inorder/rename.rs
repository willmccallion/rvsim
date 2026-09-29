//! Rename for the in-order engine: a ROB entry per instruction and
//! scoreboard tags naming the producer of each source operand.
//!
//! Source tags are captured before the destination is claimed, so an
//! instruction reading its own destination (`addi x5, x5, 16`) sees the
//! previous producer.

use super::InOrderEngine;
use crate::core::exec::inst::Inst;
use crate::core::pipeline::engine::{ExecutionEngine, Renamed};
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::vec_prf::VecPhysReg;
use crate::core::units::vpu::mem::is_vec_store;
use crate::sim::StageCtx;
use crate::trace_rename;

impl InOrderEngine {
    /// Renames one decoded instruction and allocates its ROB and store
    /// buffer slots. A store waits here, before anything is allocated, when
    /// its buffer is full; `can_accept` covers the ROB and issue-queue slots
    /// every instruction needs.
    pub(super) fn rename_one(&mut self, state: &StageCtx<'_>, id: IdExEntry) -> Renamed {
        let store_slot = if id.inst.ctrl.uses_store_buffer() {
            !self.store_buffer.is_full()
        } else if is_vec_store(id.inst.ctrl.vec_op) {
            self.vec_store_buffer.free_slots() > 0
        } else {
            true
        };
        if !store_slot {
            return Renamed::Stalled(Box::new(id));
        }
        let vector = self.vector_config(&state.hart().csrs);
        let Some(rob_tag) = self.rob.allocate(
            id.inst.pc,
            id.inst.bits,
            id.inst.size,
            id.inst.rd,
            id.inst.ctrl.fp_reg_write,
            id.inst.ctrl,
            PhysReg(0),
            PhysReg(0),
            id.seq,
        ) else {
            return Renamed::Stalled(Box::new(id));
        };

        // Capture source tags BEFORE updating scoreboard for rd.
        let rs1_tag = self.scoreboard.get_producer(id.inst.rs1, id.inst.ctrl.rs1_fp);
        let rs2_tag = self.scoreboard.get_producer(id.inst.rs2, id.inst.ctrl.rs2_fp);
        let rs3_tag = if id.inst.ctrl.rs3_fp {
            self.scoreboard.get_producer(id.inst.rs3, true)
        } else {
            None
        };

        if id.inst.ctrl.reg_write || id.inst.ctrl.fp_reg_write {
            self.scoreboard.set_producer(id.inst.rd, id.inst.ctrl.fp_reg_write, rob_tag);
        }

        let slot_allocated = if id.inst.ctrl.uses_store_buffer() {
            self.store_buffer.allocate(rob_tag, id.inst.ctrl.width)
        } else if is_vec_store(id.inst.ctrl.vec_op) {
            self.vec_store_buffer.allocate(rob_tag)
        } else {
            true
        };
        debug_assert!(slot_allocated, "the store slot was checked before allocating");

        let entry = RenameIssueEntry {
            inst: Inst {
                pc: id.inst.pc,
                bits: id.inst.bits,
                size: id.inst.size,
                rs1: id.inst.rs1,
                rs2: id.inst.rs2,
                rs3: id.inst.rs3,
                rd: id.inst.rd,
                imm: id.inst.imm,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl: id.inst.ctrl,
            },
            rob_tag,
            rs1_phys: PhysReg(0),
            rs2_phys: PhysReg(0),
            rs3_phys: PhysReg(0),
            rd_phys: PhysReg(0),
            rs1_tag,
            rs2_tag,
            rs3_tag,
            trap: id.trap,
            exception_stage: id.exception_stage,
            pred_taken: id.pred_taken,
            pred_target: id.pred_target,
            seq: id.seq,
            vs1_phys: [VecPhysReg::ZERO; 8],
            vs2_phys: [VecPhysReg::ZERO; 8],
            vs3_phys: [VecPhysReg::ZERO; 8],
            vd_phys: [VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: VecPhysReg::ZERO,
            vec_vtype: vector.vtype,
            vec_vl: vector.vl,
            vec_vstart: vector.vstart,
            vec_vxrm: state.hart().csrs.vxrm,
            vec_frm: state.hart().csrs.frm,
        };

        trace_rename!(state.config.general.trace_instructions;
            pc         = %crate::trace::Hex(entry.inst.pc),
            rob_tag    = entry.rob_tag.0,
            rd         = entry.inst.rd.as_usize(),
            rs1        = entry.inst.rs1.as_usize(),
            rs1_tag    = ?entry.rs1_tag,
            rs2        = entry.inst.rs2.as_usize(),
            rs2_tag    = ?entry.rs2_tag,
            is_store   = entry.inst.ctrl.mem_write,
            is_load    = entry.inst.ctrl.mem_read,
            "RN: in-order rename"
        );

        Renamed::Accepted(Box::new(entry))
    }
}

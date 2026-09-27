//! Rename for the in-order engine: a ROB entry per instruction and
//! scoreboard tags naming the producer of each source operand.
//!
//! Source tags are captured before the destination is claimed, so an
//! instruction reading its own destination (`addi x5, x5, 16`) sees the
//! previous producer.

use super::InOrderEngine;
use crate::core::pipeline::engine::{ExecutionEngine, Renamed};
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::units::vpu::mem::is_vec_store;
use crate::core::units::vpu::types::VecPhysReg;
use crate::sim::StageCtx;
use crate::trace_rename;

impl InOrderEngine {
    /// Renames one decoded instruction and allocates its ROB and store
    /// buffer slots; `can_accept` covers both, so this does not stall.
    pub(super) fn rename_one(&mut self, state: &StageCtx<'_>, id: IdExEntry) -> Renamed {
        let vector = self.vector_config(&state.hart().csrs);
        let Some(rob_tag) = self.rob.allocate(
            id.pc,
            id.inst,
            id.inst_size,
            id.rd,
            id.ctrl.fp_reg_write,
            id.ctrl,
            PhysReg(0),
            PhysReg(0),
            id.seq,
        ) else {
            return Renamed::Stalled(Box::new(id));
        };

        // Capture source tags BEFORE updating scoreboard for rd.
        let rs1_tag = self.scoreboard.get_producer(id.rs1, id.ctrl.rs1_fp);
        let rs2_tag = self.scoreboard.get_producer(id.rs2, id.ctrl.rs2_fp);
        let rs3_tag =
            if id.ctrl.rs3_fp { self.scoreboard.get_producer(id.rs3, true) } else { None };

        if id.ctrl.reg_write || id.ctrl.fp_reg_write {
            self.scoreboard.set_producer(id.rd, id.ctrl.fp_reg_write, rob_tag);
        }

        if id.ctrl.mem_write {
            if !self.store_buffer.allocate(rob_tag, id.ctrl.width) {
                return Renamed::Stalled(Box::new(id));
            }
        } else if is_vec_store(id.ctrl.vec_op) && !self.vec_store_buffer.allocate(rob_tag) {
            return Renamed::Stalled(Box::new(id));
        }

        let entry = RenameIssueEntry {
            rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rs1: id.rs1,
            rs2: id.rs2,
            rs3: id.rs3,
            rd: id.rd,
            imm: id.imm,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_phys: PhysReg(0),
            rs2_phys: PhysReg(0),
            rs3_phys: PhysReg(0),
            rd_phys: PhysReg(0),
            rs1_tag,
            rs2_tag,
            rs3_tag,
            ctrl: id.ctrl,
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
            pc         = %crate::trace::Hex(entry.pc),
            rob_tag    = entry.rob_tag.0,
            rd         = entry.rd.as_usize(),
            rs1        = entry.rs1.as_usize(),
            rs1_tag    = ?entry.rs1_tag,
            rs2        = entry.rs2.as_usize(),
            rs2_tag    = ?entry.rs2_tag,
            is_store   = entry.ctrl.mem_write,
            is_load    = entry.ctrl.mem_read,
            "RN: in-order rename"
        );

        Renamed::Accepted(Box::new(entry))
    }
}

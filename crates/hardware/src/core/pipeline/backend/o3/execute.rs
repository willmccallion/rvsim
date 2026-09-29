//! O3 Execute: single-instruction execution for the out-of-order backend.
//!
//! [`execute_one`] executes a single issued instruction on the logic in
//! [`shared::execute`](crate::core::pipeline::backend::shared::execute). Vector
//! ops other than vsetvl* execute in the engine, where the vector PRF is.

use crate::common::error::{ExceptionStage, Trap};
use crate::core::exec::execute::{SystemEffect, evaluate, operands, system_effect, unit_disabled};
use crate::core::exec::signals::{SystemOp, VectorOp};
use crate::core::pipeline::backend::shared::execute::{
    fault, propagate_trap, resolve_control_flow,
};
use crate::core::pipeline::backend::shared::vector_config::set_vector_config;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::squash::{Redirect, SquashCause};
use crate::sim::StageCtx;
use crate::{trace_execute, trace_trap};

/// Execute a single instruction for the O3 backend.
///
/// Returns the result and, when the instruction squashes what follows it
/// (misprediction, CSR write, MRET/SRET, FENCE.I, a fault), the
/// [`Redirect`] the engine takes after the redirect latency.
pub fn execute_one(
    state: &mut StageCtx<'_>,
    id: &RenameIssueEntry,
    rob: &mut Rob,
) -> (ExMem1Entry, Option<Redirect>) {
    if let Some(trap) = id.trap.clone() {
        return (propagate_trap(state, id, trap), None);
    }

    if state.check_execute_trigger(id.inst.pc) {
        return faulted(state, id, Trap::Breakpoint(id.inst.pc));
    }

    trace_execute!(state.config.general.trace_instructions;
        rob_tag  = id.rob_tag.0,
        pc       = %crate::trace::Hex(id.inst.pc),
        inst     = %crate::trace::Hex32(id.inst.bits),
        rd       = id.inst.rd.as_usize(),
        rd_phys  = id.rd_phys.0,
        rs1      = id.inst.rs1.as_usize(),
        rs1_phys = id.rs1_phys.0,
        rv1      = %crate::trace::Hex(id.inst.rv1),
        rs2      = id.inst.rs2.as_usize(),
        rs2_phys = id.rs2_phys.0,
        rv2      = %crate::trace::Hex(id.inst.rv2),
        imm      = id.inst.imm,
        a_src    = ?id.inst.ctrl.a_src,
        b_src    = ?id.inst.ctrl.b_src,
        alu_op   = ?id.inst.ctrl.alu,
        is_rv32  = id.inst.ctrl.is_rv32,
        is_fp    = id.inst.ctrl.fp_reg_write,
        "EX: begin"
    );

    if unit_disabled(state.hart(), &id.inst) {
        return faulted(state, id, Trap::IllegalInstruction(id.inst.bits));
    }

    let inst = &id.inst;
    let (op_a, op_b) = operands(inst);

    if id.inst.ctrl.vec_op != VectorOp::None {
        if id.inst.ctrl.vec_op.is_config() {
            let vl = set_vector_config(state, id, id.inst.rv1, id.inst.rv2, rob);
            return (ExMem1Entry::from_issue(id, vl, 0), None);
        }
        return (ExMem1Entry::from_issue(id, op_a, id.inst.rv2), None);
    }

    if let Some(executed) = execute_system(state, id, rob) {
        return executed;
    }

    let (alu_out, fp_flags) = evaluate(state, inst, op_a, op_b);
    let redirect = match resolve_control_flow(state, rob, id, op_a, op_b) {
        Ok(redirect) => redirect,
        Err(trap) => return faulted(state, id, trap),
    };
    (ExMem1Entry { fp_flags, ..ExMem1Entry::from_issue(id, alu_out, id.inst.rv2) }, redirect)
}

/// The instruction after `id` squashes and refetches.
const fn refetch_after(id: &RenameIssueEntry) -> Redirect {
    Redirect::to(id.inst.next_pc(), SquashCause::System)
}

fn faulted(
    state: &StageCtx<'_>,
    id: &RenameIssueEntry,
    trap: Trap,
) -> (ExMem1Entry, Option<Redirect>) {
    (fault(state, id, trap, ExceptionStage::Execute), None)
}

/// Executes a system instruction; `None` for everything else, FENCE
/// included (it orders memory in issue and at commit, not here).
fn execute_system(
    state: &StageCtx<'_>,
    id: &RenameIssueEntry,
    rob: &mut Rob,
) -> Option<(ExMem1Entry, Option<Redirect>)> {
    let executed = match system_effect(state, &id.inst) {
        SystemEffect::NotSystem => return None,
        SystemEffect::Trap(trap) => faulted(state, id, trap),
        // FENCE.I's I-cache flush waits for commit, so older stores are
        // visible before the refill.
        SystemEffect::AtRetire => {
            if matches!(id.inst.ctrl.system_op, SystemOp::Mret | SystemOp::Sret) {
                trace_trap!(state.config.general.trace_instructions;
                    event     = "return",
                    pc        = %crate::trace::Hex(id.inst.pc),
                    rob_tag   = id.rob_tag.0,
                    insn      = ?id.inst.ctrl.system_op,
                    priv_mode = ?state.hart().privilege,
                    mstatus   = %crate::trace::Hex(state.hart().csrs.mstatus),
                    "EX: xRET queued (privilege restore deferred to commit)"
                );
            }
            (ExMem1Entry::from_issue(id, 0, 0), Some(refetch_after(id)))
        }
        // Commit drains the store buffer, flushes the TLBs and squashes.
        SystemEffect::SfenceVma(sfence_vma) => {
            let result = ExMem1Entry {
                sfence_vma: Some(sfence_vma),
                ..ExMem1Entry::from_issue(id, 0, id.inst.rv2)
            };
            (result, None)
        }
        // A CBO passes its operand to memory1, which translates the block;
        // commit performs it. Younger loads wait for it in issue.
        SystemEffect::Cbo(_) => (ExMem1Entry::from_issue(id, id.inst.rv1, 0), None),
        // Nothing younger is renamed until this commits (serialize-after),
        // so the write needs no squash.
        SystemEffect::Csr(access) => {
            if let Some(update) = access.update {
                rob.set_csr_update(id.rob_tag, update.into());
            }
            (ExMem1Entry::from_issue(id, access.old, id.inst.rv2), None)
        }
    };
    Some(executed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::{InstSize, RegIdx};
    use crate::config::Config;
    use crate::core::exec::inst::Inst;
    use crate::core::exec::signals::{ControlFlow, ControlSignals, OpBSrc};

    #[test]
    fn test_execute_one_normal() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: 0x1000,
                bits: 0,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(1),
                imm: 0,
                rv1: 10,
                rv2: 20,
                rv3: 0,
                ctrl: ControlSignals::default(),
            },
            rob_tag: tag,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::pipeline::vec_prf::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert!(redirect.is_none());
        assert_eq!(result.alu, 10); // rv1 (10) + 0
        assert_eq!(result.rob_tag, tag);
    }

    #[test]
    fn propagated_trap_travels_with_the_result() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: 0x1000,
                bits: 0,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(1),
                imm: 0,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl: ControlSignals::default(),
            },
            rob_tag: tag,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            trap: Some(Trap::IllegalInstruction(0)),
            exception_stage: Some(ExceptionStage::Decode),
            pred_taken: false,
            pred_target: 0,
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::pipeline::vec_prf::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert!(result.trap.is_some());
        let entry = rob.find_entry(tag).unwrap();
        assert_ne!(entry.state, crate::core::pipeline::rob::RobState::Faulted);
    }

    #[test]
    fn test_execute_fence_i() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();

        let ctrl = ControlSignals { system_op: SystemOp::FenceI, ..Default::default() };

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: 0x1000,
                bits: 0,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(0),
                imm: 0,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl,
            },
            rob_tag: tag,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::pipeline::vec_prf::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert_eq!(redirect.map(|r| r.target), Some(0x1004));
    }

    #[test]
    fn fp_op_with_fs_off_carries_an_illegal_instruction_trap() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        state.hart.csrs.mstatus &= !crate::core::arch::csr::MSTATUS_FS; // Clear FS bits

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();

        let ctrl = ControlSignals {
            fp_reg_write: true, // Make it an FP instruction
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: 0x1000,
                bits: 0,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(1),
                imm: 0,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl,
            },
            rob_tag: tag,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::pipeline::vec_prf::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert_eq!(result.trap, Some(Trap::IllegalInstruction(issue.inst.bits)));
    }

    #[test]
    fn test_execute_branch_misprediction() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();

        let ctrl = ControlSignals {
            control_flow: ControlFlow::Branch,
            b_src: OpBSrc::Reg2,
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            // BEQ (funct3 = 0) with rv1 == rv2, so taken.
            inst: Inst {
                pc: 0x1000,
                bits: 0,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(0),
                imm: 8,
                rv1: 10,
                rv2: 10,
                rv3: 0,
                ctrl,
            },
            rob_tag: tag,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            // Predicted NOT taken
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::pipeline::vec_prf::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert_eq!(redirect.map(|r| r.target), Some(0x1008));
        assert!(redirect.is_some_and(|r| r.repair.is_some_and(|repair| repair.taken)));
        let entry = rob.find_entry(tag).unwrap();
        assert!(entry.bp_outcome.mispredicted);
    }

    #[test]
    fn test_execute_jump_jalr() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();

        let ctrl = ControlSignals { control_flow: ControlFlow::Jump, ..Default::default() };

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: 0x1000,
                bits: crate::isa::encoding::rv64i::opcodes::OP_JALR,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(1),
                imm: 0x15,
                rv1: 0x2000,
                rv2: 0,
                rv3: 0,
                ctrl,
            },
            rob_tag: tag,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            trap: None,
            exception_stage: None,
            pred_taken: true,
            pred_target: 0,
            // Predicted incorrectly
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::pipeline::vec_prf::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::pipeline::vec_prf::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert!(redirect.is_some());

        let expected_target = (0x2000 + 0x15) & !1;
        assert_eq!(redirect.map(|r| r.target), Some(expected_target));
    }
}

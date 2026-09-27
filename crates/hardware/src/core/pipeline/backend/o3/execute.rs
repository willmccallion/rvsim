//! O3 Execute: single-instruction execution for the out-of-order backend.
//!
//! [`execute_one`] executes a single issued instruction on the logic in
//! [`shared::execute`](crate::core::pipeline::backend::shared::execute). Vector
//! ops other than vsetvl* execute in the engine, where the vector PRF is.

use crate::common::SfenceVmaInfo;
use crate::common::error::{ExceptionStage, Trap};
use crate::core::pipeline::backend::shared::execute::{
    csr_access, ecall_trap, evaluate, fault, next_pc, operands, privileged_op_fault,
    propagate_trap, resolve_branch, resolve_jump, unit_disabled,
};
use crate::core::pipeline::backend::shared::vector_config::set_vector_config;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::signals::{ControlFlow, SystemOp, VectorOp};
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

    if state.check_execute_trigger(id.pc) {
        return faulted(state, id, Trap::Breakpoint(id.pc));
    }

    trace_execute!(state.config.general.trace_instructions;
        rob_tag  = id.rob_tag.0,
        pc       = %crate::trace::Hex(id.pc),
        inst     = %crate::trace::Hex32(id.inst),
        rd       = id.rd.as_usize(),
        rd_phys  = id.rd_phys.0,
        rs1      = id.rs1.as_usize(),
        rs1_phys = id.rs1_phys.0,
        rv1      = %crate::trace::Hex(id.rv1),
        rs2      = id.rs2.as_usize(),
        rs2_phys = id.rs2_phys.0,
        rv2      = %crate::trace::Hex(id.rv2),
        imm      = id.imm,
        a_src    = ?id.ctrl.a_src,
        b_src    = ?id.ctrl.b_src,
        alu_op   = ?id.ctrl.alu,
        is_rv32  = id.ctrl.is_rv32,
        is_fp    = id.ctrl.fp_reg_write,
        "EX: begin"
    );

    if unit_disabled(state, id) {
        return faulted(state, id, Trap::IllegalInstruction(id.inst));
    }

    let (op_a, op_b) = operands(id);

    if id.ctrl.vec_op != VectorOp::None {
        if id.ctrl.vec_op.is_config() {
            let vl = set_vector_config(state, id, id.rv1, id.rv2, rob);
            return (ExMem1Entry::from_issue(id, vl, 0), None);
        }
        return (ExMem1Entry::from_issue(id, op_a, id.rv2), None);
    }

    if let Some(executed) = execute_system(state, id, rob) {
        return executed;
    }

    let (alu_out, fp_flags) = evaluate(state, id, op_a, op_b);
    let redirect = match id.ctrl.control_flow {
        ControlFlow::Branch => resolve_branch(state, rob, id, op_a, op_b),
        ControlFlow::Jump => resolve_jump(state, rob, id),
        ControlFlow::Sequential => None,
    };
    (ExMem1Entry { fp_flags, ..ExMem1Entry::from_issue(id, alu_out, id.rv2) }, redirect)
}

/// The instruction after `id` squashes and refetches.
const fn refetch_after(id: &RenameIssueEntry) -> Redirect {
    Redirect::to(next_pc(id), SquashCause::System)
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
    if let Some(trap) = privileged_op_fault(state, id) {
        return Some(faulted(state, id, trap));
    }
    let executed = match id.ctrl.system_op {
        SystemOp::None | SystemOp::Fence => return None,
        // FENCE.I's I-cache flush waits for commit, so older stores are
        // visible before the refill.
        SystemOp::FenceI | SystemOp::Wfi => {
            (ExMem1Entry::from_issue(id, 0, 0), Some(refetch_after(id)))
        }
        SystemOp::Mret | SystemOp::Sret => {
            trace_trap!(state.config.general.trace_instructions;
                event     = "return",
                pc        = %crate::trace::Hex(id.pc),
                rob_tag   = id.rob_tag.0,
                insn      = ?id.ctrl.system_op,
                priv_mode = ?state.hart().privilege,
                mstatus   = %crate::trace::Hex(state.hart().csrs.mstatus),
                "EX: xRET queued (privilege restore deferred to commit)"
            );
            (ExMem1Entry::from_issue(id, 0, 0), Some(refetch_after(id)))
        }
        // Commit drains the store buffer, flushes the TLBs and squashes.
        SystemOp::SfenceVma => {
            let sfence_vma = SfenceVmaInfo {
                rs1_idx: id.rs1,
                rs2_idx: id.rs2,
                rs1_val: id.rv1,
                rs2_val: id.rv2,
            };
            let result = ExMem1Entry {
                sfence_vma: Some(sfence_vma),
                ..ExMem1Entry::from_issue(id, 0, id.rv2)
            };
            (result, None)
        }
        // CBO ops gate, translate and take effect at commit, which reads the
        // block address from `alu`; younger loads wait for them in issue.
        SystemOp::CboZero | SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush => {
            (ExMem1Entry::from_issue(id, id.rv1, 0), None)
        }
        SystemOp::Ecall => faulted(state, id, ecall_trap(state)),
        SystemOp::Csr => match csr_access(state, id) {
            // Only CSR writes need a flush; pure reads stay serialized at issue time.
            Ok(access) => {
                let redirect = access.update.map(|update| {
                    rob.set_csr_update(id.rob_tag, update);
                    refetch_after(id)
                });
                (ExMem1Entry::from_issue(id, access.old, id.rv2), redirect)
            }
            Err(trap) => faulted(state, id, trap),
        },
    };
    Some(executed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::{InstSize, RegIdx};
    use crate::config::Config;
    use crate::core::pipeline::signals::{ControlSignals, OpBSrc};
    use crate::core::units::bru::Ghr;
    use crate::core::units::bru::RasSnapshot;

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
            )
            .unwrap();

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 10,
            rv2: 20,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
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
            )
            .unwrap();

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl: ControlSignals::default(),
            trap: Some(Trap::IllegalInstruction(0)),
            exception_stage: Some(ExceptionStage::Decode),
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
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
            )
            .unwrap();

        let ctrl = ControlSignals { system_op: SystemOp::FenceI, ..Default::default() };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(0),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
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
            )
            .unwrap();

        let ctrl = ControlSignals {
            fp_reg_write: true, // Make it an FP instruction
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert_eq!(result.trap, Some(Trap::IllegalInstruction(issue.inst)));
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
            )
            .unwrap();

        let ctrl = ControlSignals {
            control_flow: ControlFlow::Branch,
            b_src: OpBSrc::Reg2,
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: (0 << 12),
            inst_size: InstSize::Standard, // BEQ (funct3 = 0)
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(0),
            imm: 8,
            rv1: 10,
            rv2: 10,
            rv3: 0, // rv1 == rv2, so taken
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0, // Predicted NOT taken
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
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
            )
            .unwrap();

        let ctrl = ControlSignals { control_flow: ControlFlow::Jump, ..Default::default() };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: crate::isa::rv64i::opcodes::OP_JALR,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0x15,
            rv1: 0x2000,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: true,
            pred_target: 0, // Predicted incorrectly
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
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

//! O3 Execute: single-instruction execution for the out-of-order backend.
//!
//! [`execute_one`] executes a single issued instruction on the logic in
//! [`shared::execute`](crate::uarch::pipeline::backend::shared::execute). Vector
//! ops other than vsetvl* execute in the engine, where the vector PRF is.

use super::O3Entry;
use crate::exec::execute::{evaluate, illegal_in_current_state, operands, perform_csr_access};
use crate::isa::op::{SystemOp, VectorOp};
use crate::isa::privileged::Trap;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::execute::{
    SystemExecution, execute_system, faulted, propagate_trap, refetch_after, resolve_control_flow,
};
use crate::uarch::pipeline::backend::shared::vector_config::set_vector_config;
use crate::uarch::pipeline::latches::ExMem1Entry;
use crate::uarch::pipeline::rob::Rob;
use crate::uarch::pipeline::squash::Redirect;
use crate::uarch::pipeline::trace::Stage;
use crate::{trace_execute, trace_trap};

/// Execute a single instruction for the O3 backend.
///
/// Returns the result and, when the instruction squashes what follows it
/// (misprediction, CSR write, MRET/SRET, FENCE.I, a fault), the
/// [`Redirect`] the engine takes after the redirect latency.
pub fn execute_one(
    state: &mut StageCtx<'_>,
    issued: &O3Entry,
    rob: &mut Rob,
) -> (ExMem1Entry, Option<Redirect>) {
    if let Some(trap) = issued.trap.clone() {
        return (propagate_trap(state, issued, trap), None);
    }

    if state.check_execute_trigger(issued.inst.pc) {
        return faulted(state, issued, Trap::Breakpoint(issued.inst.pc.val()));
    }
    state.trace_stage(issued.seq, Stage::Execute);

    trace_execute!(state.config.general.trace_instructions;
        rob_tag  = %issued.rob_tag,
        pc       = %crate::common::trace::Hex(issued.inst.pc.val()),
        inst     = %crate::common::trace::Hex32(issued.inst.bits),
        rd       = issued.inst.rd.as_usize(),
        rd_phys  = issued.regs.rd_phys.index(),
        rs1      = issued.inst.rs1.as_usize(),
        rs1_phys = issued.regs.rs1_phys.index(),
        rv1      = %crate::common::trace::Hex(issued.inst.rv1),
        rs2      = issued.inst.rs2.as_usize(),
        rs2_phys = issued.regs.rs2_phys.index(),
        rv2      = %crate::common::trace::Hex(issued.inst.rv2),
        imm      = issued.inst.imm,
        a_src    = ?issued.inst.ctrl.a_src,
        b_src    = ?issued.inst.ctrl.b_src,
        alu_op   = ?issued.inst.ctrl.alu,
        format   = ?issued.inst.ctrl.format,
        is_fp    = issued.inst.ctrl.fp_reg_write,
        "EX: begin"
    );

    if illegal_in_current_state(state.hart(), &issued.inst) {
        return faulted(state, issued, Trap::IllegalInstruction(issued.inst.bits));
    }

    if let Some(executed) = execute_system_o3(state, issued, rob) {
        return executed;
    }

    let inst = &issued.inst;
    let (op_a, op_b) = operands(inst);

    if issued.inst.ctrl.vec_op != VectorOp::None {
        if issued.inst.ctrl.vec_op.is_config() {
            let vl = set_vector_config(state, issued, issued.inst.rv1, issued.inst.rv2, rob);
            return (ExMem1Entry::from_issue(issued, vl, 0), None);
        }
        return (ExMem1Entry::from_issue(issued, op_a, issued.inst.rv2), None);
    }

    let (alu_out, fp_flags) = evaluate(state, inst, op_a, op_b);
    let redirect = match resolve_control_flow(state, rob, issued, op_a, op_b) {
        Ok(redirect) => redirect,
        Err(trap) => return faulted(state, issued, trap),
    };
    (
        ExMem1Entry { fp_flags, ..ExMem1Entry::from_issue(issued, alu_out, issued.inst.rv2) },
        redirect,
    )
}

/// Executes a system instruction; `None` for everything else. FENCE.I,
/// MRET, SRET and WFI refetch what follows them at once, and FENCE.I's
/// I-cache flush waits for commit, so older stores are visible before the
/// refill. A CSR access is performed here: rename held everything younger
/// until the ROB drained (serialize-after), so it reads what every older
/// instruction wrote, and the write needs no squash.
fn execute_system_o3(
    state: &StageCtx<'_>,
    issued: &O3Entry,
    rob: &mut Rob,
) -> Option<(ExMem1Entry, Option<Redirect>)> {
    Some(match execute_system(state, issued)? {
        SystemExecution::Done(result, redirect) => (result, redirect),
        SystemExecution::AtRetire => {
            if matches!(issued.inst.ctrl.system_op, SystemOp::Mret | SystemOp::Sret) {
                trace_trap!(state.config.general.trace_instructions;
                    event     = "return",
                    pc        = %crate::common::trace::Hex(issued.inst.pc.val()),
                    rob_tag   = %issued.rob_tag,
                    insn      = ?issued.inst.ctrl.system_op,
                    priv_mode = ?state.hart().privilege,
                    mstatus   = %crate::common::trace::Hex(state.hart().csrs.mstatus),
                    "EX: xRET queued (privilege restore deferred to commit)"
                );
            }
            (ExMem1Entry::from_issue(issued, 0, 0), Some(refetch_after(issued)))
        }
        SystemExecution::Csr(request) => {
            let access = perform_csr_access(state, &request);
            if let Some(update) = access.update {
                rob.set_csr_update(issued.rob_tag, update.into());
            }
            (ExMem1Entry::from_issue(issued, access.old, issued.inst.rv2), None)
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::VirtAddr;
    use crate::config::Config;
    use crate::exec::inst::Inst;
    use crate::exec::signals::{ControlFlow, ControlSignals, OpBSrc};
    use crate::isa::instruction::InstSize;
    use crate::isa::reg::RegIdx;
    use crate::uarch::pipeline::backend::o3::RenamedRegisters;
    use crate::uarch::pipeline::latches::RenameIssueEntry;

    #[test]
    fn test_execute_one_normal() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                &crate::exec::inst::Inst {
                    pc: VirtAddr::new(0x1000),
                    bits: 0,
                    size: InstSize::Standard,
                    rd: RegIdx::new(1),
                    ctrl: ControlSignals::default(),
                    ..Default::default()
                },
                crate::common::InstSeq::default(),
                crate::uarch::pipeline::rob::RenamedDst::NONE,
            )
            .unwrap();

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: VirtAddr::new(0x1000),
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
            trap: None,
            predicted_target: None,
            seq: crate::common::InstSeq::default(),
            vector_config: crate::isa::rvv::VectorConfig::default(),
            vector_rounding: crate::isa::rvv::VectorRounding::default(),
            regs: RenamedRegisters::default(),
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert!(redirect.is_none());
        assert_eq!(result.alu, 10); // rv1 (10) + 0
        assert_eq!(result.rob_tag, tag);
    }

    #[test]
    fn propagated_trap_travels_with_the_result() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                &crate::exec::inst::Inst {
                    pc: VirtAddr::new(0x1000),
                    bits: 0,
                    size: InstSize::Standard,
                    rd: RegIdx::new(1),
                    ctrl: ControlSignals::default(),
                    ..Default::default()
                },
                crate::common::InstSeq::default(),
                crate::uarch::pipeline::rob::RenamedDst::NONE,
            )
            .unwrap();

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: VirtAddr::new(0x1000),
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
            trap: Some(Trap::IllegalInstruction(0)),
            predicted_target: None,
            seq: crate::common::InstSeq::default(),
            vector_config: crate::isa::rvv::VectorConfig::default(),
            vector_rounding: crate::isa::rvv::VectorRounding::default(),
            regs: RenamedRegisters::default(),
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert!(result.trap.is_some());
        let entry = rob.find_entry(tag).unwrap();
        assert!(!entry.state.is_faulted());
    }

    #[test]
    fn test_execute_fence_i() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                &crate::exec::inst::Inst {
                    pc: VirtAddr::new(0x1000),
                    bits: 0,
                    size: InstSize::Standard,
                    rd: RegIdx::new(1),
                    ctrl: ControlSignals::default(),
                    ..Default::default()
                },
                crate::common::InstSeq::default(),
                crate::uarch::pipeline::rob::RenamedDst::NONE,
            )
            .unwrap();

        let ctrl = ControlSignals { system_op: SystemOp::FenceI, ..Default::default() };

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: VirtAddr::new(0x1000),
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
            trap: None,
            predicted_target: None,
            seq: crate::common::InstSeq::default(),
            vector_config: crate::isa::rvv::VectorConfig::default(),
            vector_rounding: crate::isa::rvv::VectorRounding::default(),
            regs: RenamedRegisters::default(),
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert_eq!(redirect.map(|r| r.target), Some(VirtAddr::new(0x1004)));
    }

    #[test]
    fn fp_op_with_fs_off_carries_an_illegal_instruction_trap() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        state.hart.csrs.mstatus &= !crate::isa::csr::MSTATUS_FS; // Clear FS bits

        let tag = rob
            .allocate(
                &crate::exec::inst::Inst {
                    pc: VirtAddr::new(0x1000),
                    bits: 0,
                    size: InstSize::Standard,
                    rd: RegIdx::new(1),
                    ctrl: ControlSignals::default(),
                    ..Default::default()
                },
                crate::common::InstSeq::default(),
                crate::uarch::pipeline::rob::RenamedDst::NONE,
            )
            .unwrap();

        let ctrl = ControlSignals {
            fp_reg_write: true, // Make it an FP instruction
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: VirtAddr::new(0x1000),
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
            trap: None,
            predicted_target: None,
            seq: crate::common::InstSeq::default(),
            vector_config: crate::isa::rvv::VectorConfig::default(),
            vector_rounding: crate::isa::rvv::VectorRounding::default(),
            regs: RenamedRegisters::default(),
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert_eq!(result.trap, Some(Trap::IllegalInstruction(issue.inst.bits)));
    }

    #[test]
    fn test_execute_branch_misprediction() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                &crate::exec::inst::Inst {
                    pc: VirtAddr::new(0x1000),
                    bits: 0,
                    size: InstSize::Standard,
                    rd: RegIdx::new(1),
                    ctrl: ControlSignals::default(),
                    ..Default::default()
                },
                crate::common::InstSeq::default(),
                crate::uarch::pipeline::rob::RenamedDst::NONE,
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
                pc: VirtAddr::new(0x1000),
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
            trap: None,
            predicted_target: None,
            // Predicted NOT taken
            seq: crate::common::InstSeq::default(),
            vector_config: crate::isa::rvv::VectorConfig::default(),
            vector_rounding: crate::isa::rvv::VectorRounding::default(),
            regs: RenamedRegisters::default(),
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert_eq!(redirect.map(|r| r.target), Some(VirtAddr::new(0x1008)));
        assert!(redirect.is_some_and(|r| r.repair.is_some_and(|repair| repair.taken)));
        let entry = rob.find_entry(tag).unwrap();
        assert!(entry.control.is_some_and(|control| control.outcome.mispredicted));
    }

    #[test]
    fn test_execute_jump_jalr() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                &crate::exec::inst::Inst {
                    pc: VirtAddr::new(0x1000),
                    bits: 0,
                    size: InstSize::Standard,
                    rd: RegIdx::new(1),
                    ctrl: ControlSignals::default(),
                    ..Default::default()
                },
                crate::common::InstSeq::default(),
                crate::uarch::pipeline::rob::RenamedDst::NONE,
            )
            .unwrap();

        let ctrl = ControlSignals { control_flow: ControlFlow::Jump, ..Default::default() };

        let issue = RenameIssueEntry {
            inst: Inst {
                pc: VirtAddr::new(0x1000),
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
            trap: None,
            predicted_target: Some(VirtAddr::new(0)),
            // Predicted incorrectly
            seq: crate::common::InstSeq::default(),
            vector_config: crate::isa::rvv::VectorConfig::default(),
            vector_rounding: crate::isa::rvv::VectorRounding::default(),
            regs: RenamedRegisters::default(),
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);
        assert!(redirect.is_some());

        let expected_target = (0x2000 + 0x15) & !1;
        assert_eq!(redirect.map(|r| r.target), Some(VirtAddr::new(expected_target)));
    }
}

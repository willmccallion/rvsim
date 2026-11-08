//! O3 Execute: single-instruction execution for the out-of-order backend.
//!
//! [`execute_one`] executes a single issued instruction on the logic in
//! [`shared::execute`](crate::uarch::pipeline::backend::shared::execute). Vector
//! ops other than vsetvl* execute in the engine, where the vector PRF is.

use super::O3Entry;
use crate::common::trace::Hex;
use crate::common::trace::Hex32;
use crate::exec::execute::{evaluate, illegal_in_current_state, operands, perform_csr_access};
use crate::isa::op::SystemOp;
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

    trace_execute!(state.uncore().kernel.config.general.trace_instructions;
        rob_tag  = %issued.rob_tag,
        pc       = %Hex(issued.inst.pc.val()),
        inst     = %Hex32(issued.inst.bits),
        rd       = issued.inst.rd.as_usize(),
        rd_phys  = issued.regs.rd_phys.index(),
        rs1      = issued.inst.rs1.as_usize(),
        rs1_phys = issued.regs.rs1_phys.index(),
        rv1      = %Hex(issued.inst.rv1),
        rs2      = issued.inst.rs2.as_usize(),
        rs2_phys = issued.regs.rs2_phys.index(),
        rv2      = %Hex(issued.inst.rv2),
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

    if issued.inst.ctrl.vec_op.is_some() {
        if issued.inst.ctrl.is_vector_config() {
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
    let mut result = ExMem1Entry::from_issue(issued, alu_out, issued.inst.rv2);
    result.op.fp_flags = fp_flags;
    (result, redirect)
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
        SystemExecution::Done(result) => (result, None),
        SystemExecution::AtRetire => {
            if matches!(issued.inst.ctrl.system_op, SystemOp::Mret | SystemOp::Sret) {
                trace_trap!(state.uncore().kernel.config.general.trace_instructions;
                    event     = "return",
                    pc        = %Hex(issued.inst.pc.val()),
                    rob_tag   = %issued.rob_tag,
                    insn      = ?issued.inst.ctrl.system_op,
                    priv_mode = ?state.hart().privilege,
                    mstatus   = %Hex(state.hart().csrs.mstatus),
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
    use crate::common::{InstSeq, VirtAddr};
    use crate::config::Config;
    use crate::exec::inst::Inst;
    use crate::exec::signals::{BranchCond, ControlFlow, ControlSignals, JumpKind, OpBSrc};
    use crate::isa::csr::MSTATUS_FS;
    use crate::isa::encoding::rv64i::opcodes::OP_JALR;
    use crate::isa::reg::RegIdx;
    use crate::system::SystemState;
    use crate::uarch::pipeline::latches::RenameIssueEntry;
    use crate::uarch::pipeline::rob::RenamedDst;

    const PC: u64 = 0x1000;

    /// An instruction at `PC` writing x1 under `ctrl`.
    fn inst(ctrl: ControlSignals) -> Inst {
        Inst { pc: VirtAddr::new(PC), rd: RegIdx::new(1), ctrl, ..Inst::default() }
    }

    /// `inst` allocated in `rob`, as issue hands it to execute.
    fn issued(rob: &mut Rob, inst: Inst) -> O3Entry {
        let rob_tag = rob.allocate(&inst, InstSeq::default(), RenamedDst::NONE).unwrap();
        RenameIssueEntry { rob_tag, inst, ..RenameIssueEntry::default() }
    }

    #[test]
    fn a_plain_alu_op_yields_its_result_without_a_redirect() {
        let mut sys = SystemState::build(&Config::minimal());
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        let issue = issued(&mut rob, Inst { rv1: 10, rv2: 20, ..inst(ControlSignals::default()) });

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert_eq!((result.op.alu, result.op.rob_tag), (10, issue.rob_tag), "rv1 + imm 0");
    }

    #[test]
    fn propagated_trap_travels_with_the_result() {
        let mut sys = SystemState::build(&Config::minimal());
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        let issue = O3Entry {
            trap: Some(Trap::IllegalInstruction(0)),
            ..issued(&mut rob, inst(ControlSignals::default()))
        };

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert!(result.op.trap.is_some());
        assert!(!rob.find_entry(issue.rob_tag).unwrap().state.is_faulted());
    }

    #[test]
    fn fence_i_redirects_to_the_next_instruction() {
        let mut sys = SystemState::build(&Config::minimal());
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        let fence_i = ControlSignals { system_op: SystemOp::FenceI, ..Default::default() };
        let issue = issued(&mut rob, Inst { rd: RegIdx::new(0), ..inst(fence_i) });

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert_eq!(redirect.map(|r| r.target), Some(VirtAddr::new(PC + 4)));
    }

    #[test]
    fn fp_op_with_fs_off_carries_an_illegal_instruction_trap() {
        let mut sys = SystemState::build(&Config::minimal());
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        state.hart.csrs.mstatus &= !MSTATUS_FS;
        let fp_write = ControlSignals { fp_reg_write: true, ..Default::default() };
        let issue = issued(&mut rob, inst(fp_write));

        let (result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert!(redirect.is_none());
        assert_eq!(result.op.trap, Some(Trap::IllegalInstruction(issue.inst.bits)));
    }

    #[test]
    fn a_mispredicted_taken_branch_redirects_and_records_the_misprediction() {
        let mut sys = SystemState::build(&Config::minimal());
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        let beq = ControlSignals {
            control_flow: ControlFlow::Branch(BranchCond::Eq),
            b_src: OpBSrc::Reg2,
            ..Default::default()
        };
        // Equal operands, so taken; predicted not taken.
        let issue =
            issued(&mut rob, Inst { rd: RegIdx::new(0), imm: 8, rv1: 10, rv2: 10, ..inst(beq) });

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert_eq!(redirect.map(|r| r.target), Some(VirtAddr::new(PC + 8)));
        assert!(redirect.is_some_and(|r| r.repair.is_some_and(|repair| repair.taken)));
        let entry = rob.find_entry(issue.rob_tag).unwrap();
        assert!(entry.control.is_some_and(|control| control.outcome.mispredicted));
    }

    #[test]
    fn a_mispredicted_jalr_redirects_to_its_target_with_bit_zero_cleared() {
        let mut sys = SystemState::build(&Config::minimal());
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        let jalr = ControlSignals {
            control_flow: ControlFlow::Jump(JumpKind::Jalr),
            ..Default::default()
        };
        let issue = O3Entry {
            predicted_target: Some(VirtAddr::new(0)),
            ..issued(&mut rob, Inst { bits: OP_JALR, imm: 0x15, rv1: 0x2000, ..inst(jalr) })
        };

        let (_result, redirect) = execute_one(&mut state.stage(), &issue, &mut rob);

        assert_eq!(redirect.map(|r| r.target), Some(VirtAddr::new((0x2000 + 0x15) & !1)));
    }
}

//! Execute-stage bookkeeping both backends share.
//!
//! Recording faults on results and resolving branches against their
//! predictions. What an instruction computes is in
//! [`crate::exec::execute`].

use crate::exec::compute::vector::context::VecExecResult;
use crate::exec::execute::{
    CsrRequest, SystemEffect, branch_taken, check_target_alignment, is_jalr, jump_target,
    system_effect,
};
use crate::exec::signals::ControlFlow;
use crate::isa::privileged::Trap;
use crate::isa::reg;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::uarch::pipeline::rob::{BpOutcome, Rob, RobTag};
use crate::uarch::pipeline::squash::{BranchRepair, Redirect, SquashCause};
use crate::{trace_branch, trace_trap};

/// The result of an instruction that raised `trap`. The trap
/// travels with it: writeback records it on the ROB entry and commit takes
/// it, flushing everything younger, as a real core does.
pub fn fault(state: &StageCtx<'_>, issued: &RenameIssueEntry, trap: Trap) -> ExMem1Entry {
    trace_trap!(state.trace_trap_enabled(&trap);
        event   = "fault",
        pc      = %crate::common::trace::Hex(issued.inst.pc.val()),
        rob_tag = %issued.rob_tag,
        trap    = ?trap,
        "EX: instruction faulted"
    );
    ExMem1Entry { trap: Some(trap), ..ExMem1Entry::from_issue(issued, 0, 0) }
}

/// [`fault`] with no redirect: commit takes the trap.
pub fn faulted(
    state: &StageCtx<'_>,
    issued: &RenameIssueEntry,
    trap: Trap,
) -> (ExMem1Entry, Option<Redirect>) {
    (fault(state, issued, trap), None)
}

/// The instruction after `id` squashes and refetches.
pub const fn refetch_after(issued: &RenameIssueEntry) -> Redirect {
    Redirect::to(issued.inst.next_pc(), SquashCause::System)
}

/// A system instruction's execution, or the two kinds the backends treat
/// differently: what takes effect at retire and a CSR access.
#[derive(Debug)]
pub enum SystemExecution {
    /// Executed alike on both backends.
    Done(ExMem1Entry, Option<Redirect>),
    /// FENCE.I, MRET, SRET or WFI, which take effect at commit.
    AtRetire,
    /// A CSR access.
    Csr(CsrRequest),
}

/// Executes a system instruction as far as both backends agree; `None`
/// for everything else, FENCE included (it orders memory in issue and at
/// commit, not here).
pub fn execute_system(state: &StageCtx<'_>, issued: &RenameIssueEntry) -> Option<SystemExecution> {
    Some(match system_effect(state, &issued.inst) {
        SystemEffect::NotSystem => return None,
        SystemEffect::Trap(trap) => SystemExecution::Done(fault(state, issued, trap), None),
        SystemEffect::AtRetire => SystemExecution::AtRetire,
        // Commit drains the store buffer, flushes the TLBs and squashes.
        SystemEffect::SfenceVma(sfence_vma) => SystemExecution::Done(
            ExMem1Entry {
                sfence_vma: Some(sfence_vma),
                ..ExMem1Entry::from_issue(issued, 0, issued.inst.rv2)
            },
            None,
        ),
        // A CBO passes its operand to memory1, which translates the block;
        // commit performs it. Younger loads wait for it in issue.
        SystemEffect::Cbo => {
            SystemExecution::Done(ExMem1Entry::from_issue(issued, issued.inst.rv1, 0), None)
        }
        SystemEffect::Csr(request) => SystemExecution::Csr(request),
    })
}

/// Files on `tag`'s ROB entry the FP flags and saturation a vector op
/// raised.
pub fn record_vector_flags(rob: &mut Rob, tag: RobTag, result: &VecExecResult) {
    if !result.fp_flags.is_empty() {
        rob.set_fp_flags(tag, result.fp_flags);
    }
    if result.vxsat {
        rob.set_vxsat(tag, true);
    }
}

/// The result of an instruction that arrived carrying a trap from an
/// earlier stage.
pub fn propagate_trap(state: &StageCtx<'_>, issued: &RenameIssueEntry, trap: Trap) -> ExMem1Entry {
    fault(state, issued, trap)
}

/// Resolves a branch or jump to the redirect a misprediction needs.
///
/// A sequential instruction resolves to nothing.
///
/// # Errors
///
/// The instruction-address-misaligned trap a taken branch or jump raises
/// when its target is not IALIGN-aligned.
pub fn resolve_control_flow(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    issued: &RenameIssueEntry,
    op_a: u64,
    op_b: u64,
) -> Result<Option<Redirect>, Trap> {
    match issued.inst.ctrl.control_flow {
        ControlFlow::Branch => resolve_branch(state, rob, issued, op_a, op_b),
        ControlFlow::Jump => resolve_jump(state, rob, issued),
        ControlFlow::Sequential => Ok(None),
    }
}

/// Resolves a conditional branch against its prediction, files the outcome
/// for the predictor to learn from at commit, and returns the redirect a
/// misprediction needs.
fn resolve_branch(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    issued: &RenameIssueEntry,
    op_a: u64,
    op_b: u64,
) -> Result<Option<Redirect>, Trap> {
    let taken = branch_taken(issued.inst.bits, op_a, op_b);
    let actual_target = issued.inst.pc.wrapping_add(issued.inst.imm as u64);
    let fallthrough = issued.inst.next_pc();
    let predicted_next_pc = issued.predicted_target.unwrap_or(fallthrough);
    let actual_next_pc = if taken { actual_target } else { fallthrough };
    if taken {
        check_target_alignment(state.hart(), actual_target)?;
    }
    let mispredicted = predicted_next_pc != actual_next_pc;

    rob.set_control_outcome(
        issued.rob_tag,
        BpOutcome { taken, mispredicted },
        taken.then_some(actual_target),
    );
    trace_branch!(state.config.general.trace_instructions;
        event          = "resolve",
        pc             = %crate::common::trace::Hex(issued.inst.pc.val()),
        rob_tag        = %issued.rob_tag,
        pred_taken     = issued.predicted_target.is_some(),
        pred_target    = %crate::common::trace::Hex(predicted_next_pc.val()),
        actual_taken   = taken,
        actual_target  = %crate::common::trace::Hex(actual_next_pc.val()),
        mispredicted,
        "EX: branch resolved"
    );
    let repair = BranchRepair { seq: issued.seq, taken, target: actual_target };
    Ok(count_prediction(state, mispredicted).then(|| Redirect::mispredict(actual_next_pc, repair)))
}

/// Resolves a JAL or JALR against its predicted target and returns the
/// redirect a misprediction needs. Jumps do not train the direction tables,
/// but commit counts their predictions with the branches'.
fn resolve_jump(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    issued: &RenameIssueEntry,
) -> Result<Option<Redirect>, Trap> {
    let inst = &issued.inst;
    let is_jalr = is_jalr(inst);
    let actual_target = jump_target(inst);
    check_target_alignment(state.hart(), actual_target)?;
    let predicted_target = issued.predicted_target.unwrap_or_else(|| issued.inst.next_pc());
    let mispredicted = actual_target != predicted_target;

    rob.set_control_outcome(
        issued.rob_tag,
        BpOutcome { taken: true, mispredicted },
        Some(actual_target),
    );
    let rd_link = issued.inst.rd == reg::REG_RA || issued.inst.rd == reg::REG_T0;
    let rs1_link = is_jalr && (issued.inst.rs1 == reg::REG_RA || issued.inst.rs1 == reg::REG_T0);
    trace_branch!(state.config.general.trace_instructions;
        event          = "resolve",
        pc             = %crate::common::trace::Hex(issued.inst.pc.val()),
        rob_tag        = %issued.rob_tag,
        bp_type        = if rs1_link && !rd_link { "JALR/RAS" } else if rd_link { "JAL/call" } else { "JAL/JALR" },
        pred_taken     = issued.predicted_target.is_some(),
        pred_target    = %crate::common::trace::Hex(predicted_target.val()),
        actual_taken   = true,
        actual_target  = %crate::common::trace::Hex(actual_target.val()),
        mispredicted,
        "EX: jump resolved"
    );
    let repair = BranchRepair { seq: issued.seq, taken: true, target: actual_target };
    Ok(count_prediction(state, mispredicted).then(|| Redirect::mispredict(actual_target, repair)))
}

/// Counts a resolved prediction and passes `mispredicted` through.
fn count_prediction(state: &mut StageCtx<'_>, mispredicted: bool) -> bool {
    let paths = &state.core().stat_paths.bp;
    let path = if mispredicted { paths.spec_mispredicts } else { paths.spec_hits };
    state.counter(path).inc();
    mispredicted
}

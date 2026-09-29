//! Execute-stage bookkeeping both backends share.
//!
//! Recording faults on results and resolving branches against their
//! predictions. What an instruction computes is in
//! [`crate::exec::execute`].

use crate::core::pipeline::exception::ExceptionStage;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::{BpOutcome, Rob};
use crate::core::pipeline::squash::{BranchRepair, Redirect};
use crate::exec::execute::{branch_taken, check_target_alignment, is_jalr, jump_target};
use crate::exec::signals::ControlFlow;
use crate::isa::privileged::Trap;
use crate::isa::reg;
use crate::sim::StageCtx;
use crate::{trace_branch, trace_trap};

/// The result of an instruction that raised `trap` in `stage`. The trap
/// travels with it: writeback records it on the ROB entry and commit takes
/// it, flushing everything younger, as a real core does.
pub fn fault(
    state: &StageCtx<'_>,
    id: &RenameIssueEntry,
    trap: Trap,
    stage: ExceptionStage,
) -> ExMem1Entry {
    trace_trap!(state.trace_trap_enabled(&trap);
        event   = "fault",
        stage   = ?stage,
        pc      = %crate::trace::Hex(id.inst.pc),
        rob_tag = id.rob_tag.0,
        trap    = ?trap,
        "EX: instruction faulted"
    );
    ExMem1Entry {
        trap: Some(trap),
        exception_stage: Some(stage),
        ..ExMem1Entry::from_issue(id, 0, 0)
    }
}

/// The result of an instruction that arrived carrying a trap from an
/// earlier stage.
pub fn propagate_trap(state: &StageCtx<'_>, id: &RenameIssueEntry, trap: Trap) -> ExMem1Entry {
    fault(state, id, trap, id.exception_stage.unwrap_or(ExceptionStage::Execute))
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
    id: &RenameIssueEntry,
    op_a: u64,
    op_b: u64,
) -> Result<Option<Redirect>, Trap> {
    match id.inst.ctrl.control_flow {
        ControlFlow::Branch => resolve_branch(state, rob, id, op_a, op_b),
        ControlFlow::Jump => resolve_jump(state, rob, id),
        ControlFlow::Sequential => Ok(None),
    }
}

/// Resolves a conditional branch against its prediction, files the outcome
/// for the predictor to learn from at commit, and returns the redirect a
/// misprediction needs.
fn resolve_branch(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    id: &RenameIssueEntry,
    op_a: u64,
    op_b: u64,
) -> Result<Option<Redirect>, Trap> {
    let taken = branch_taken(id.inst.bits, op_a, op_b);
    let actual_target = id.inst.pc.wrapping_add(id.inst.imm as u64);
    let fallthrough = id.inst.next_pc();
    let predicted_next_pc = if id.pred_taken { id.pred_target } else { fallthrough };
    let actual_next_pc = if taken { actual_target } else { fallthrough };
    if taken {
        check_target_alignment(state.hart(), actual_target)?;
    }
    let mispredicted = predicted_next_pc != actual_next_pc;

    rob.set_control_outcome(
        id.rob_tag,
        BpOutcome { taken, mispredicted },
        taken.then_some(actual_target),
    );
    trace_branch!(state.config.general.trace_instructions;
        event          = "resolve",
        pc             = %crate::trace::Hex(id.inst.pc),
        rob_tag        = id.rob_tag.0,
        pred_taken     = id.pred_taken,
        pred_target    = %crate::trace::Hex(predicted_next_pc),
        actual_taken   = taken,
        actual_target  = %crate::trace::Hex(actual_next_pc),
        mispredicted,
        "EX: branch resolved"
    );
    let repair = BranchRepair { seq: id.seq, taken, target: actual_target };
    Ok(count_prediction(state, mispredicted).then(|| Redirect::mispredict(actual_next_pc, repair)))
}

/// Resolves a JAL or JALR against its predicted target and returns the
/// redirect a misprediction needs. Jumps do not train the direction tables,
/// but commit counts their predictions with the branches'.
fn resolve_jump(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    id: &RenameIssueEntry,
) -> Result<Option<Redirect>, Trap> {
    let inst = &id.inst;
    let is_jalr = is_jalr(inst);
    let actual_target = jump_target(inst);
    check_target_alignment(state.hart(), actual_target)?;
    let predicted_target = if id.pred_taken { id.pred_target } else { id.inst.next_pc() };
    let mispredicted = actual_target != predicted_target;

    rob.set_control_outcome(
        id.rob_tag,
        BpOutcome { taken: true, mispredicted },
        Some(actual_target),
    );
    let rd_link = id.inst.rd == reg::REG_RA || id.inst.rd == reg::REG_T0;
    let rs1_link = is_jalr && (id.inst.rs1 == reg::REG_RA || id.inst.rs1 == reg::REG_T0);
    trace_branch!(state.config.general.trace_instructions;
        event          = "resolve",
        pc             = %crate::trace::Hex(id.inst.pc),
        rob_tag        = id.rob_tag.0,
        bp_type        = if rs1_link && !rd_link { "JALR/RAS" } else if rd_link { "JAL/call" } else { "JAL/JALR" },
        pred_taken     = id.pred_taken,
        pred_target    = %crate::trace::Hex(predicted_target),
        actual_taken   = true,
        actual_target  = %crate::trace::Hex(actual_target),
        mispredicted,
        "EX: jump resolved"
    );
    let repair = BranchRepair { seq: id.seq, taken: true, target: actual_target };
    Ok(count_prediction(state, mispredicted).then(|| Redirect::mispredict(actual_target, repair)))
}

/// Counts a resolved prediction and passes `mispredicted` through.
fn count_prediction(state: &mut StageCtx<'_>, mispredicted: bool) -> bool {
    let paths = &state.core().stat_paths.bp;
    let path = if mispredicted { paths.spec_mispredicts } else { paths.spec_hits };
    state.counter(path).inc();
    mispredicted
}

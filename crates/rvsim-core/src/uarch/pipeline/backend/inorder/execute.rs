//! In-Order Execute Unit: single ALU/FPU/BRU execution.
//!
//! This stage performs arithmetic, branch resolution, and system instruction
//! handling. CSR writes and MRET/SRET are deferred to commit via the ROB.

use super::issue::InOrderEntry;
use crate::exec::compute::vector::agnostic::AgnosticRecorder;
use crate::exec::compute::vector::execute::execute_vec_op_on;
use crate::exec::compute::vector::shadow::ShadowVpr;
use crate::exec::execute::{evaluate, illegal_in_current_state, operands};
use crate::isa::extension::Extension;
use crate::isa::op::VectorOp;
use crate::isa::privileged::Trap;
use crate::soc::uncore::commit_log_open;
use crate::trace_execute;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::execute::{
    SystemExecution, execute_system, faulted, propagate_trap, record_vector_flags, refetch_after,
    resolve_control_flow,
};
use crate::uarch::pipeline::backend::shared::vector_config::set_vector_config;
use crate::uarch::pipeline::latches::ExMem1Entry;
use crate::uarch::pipeline::rob::{Rob, RobTag};
use crate::uarch::pipeline::squash::Redirect;
use crate::uarch::pipeline::trace::Stage;

/// What one cycle of in-order execute produced.
#[derive(Debug, Default)]
pub struct ExecutedBatch {
    /// The results, in issue order.
    pub results: Vec<ExMem1Entry>,
    /// The redirect each squashing instruction asked for, by ROB tag.
    pub redirects: Vec<(RobTag, Redirect)>,
}

/// Executes instructions in the in-order backend.
///
/// Takes issued instructions, performs ALU/FPU operations, resolves branches,
/// and produces `ExMem1Entry` results. CSR writes and `MRET`/`SRET` are recorded
/// in the ROB for deferred application at commit.
///
/// Returns the results and the redirects the batch asked for (branch
/// misprediction, CSR access, MRET/SRET, FENCE.I, a vector op, a fault); the
/// engine takes each after the redirect latency.
pub fn execute_inorder(
    state: &mut StageCtx<'_>,
    entries: &[InOrderEntry],
    rob: &mut Rob,
) -> ExecutedBatch {
    let mut batch =
        ExecutedBatch { results: Vec::with_capacity(entries.len()), redirects: Vec::new() };
    for issued in entries {
        let (result, redirect) = execute_one(state, issued, rob);
        batch.results.push(result);
        batch.redirects.extend(redirect.map(|r| (issued.rob_tag, r)));
    }
    batch
}

fn execute_one(
    state: &mut StageCtx<'_>,
    issued: &InOrderEntry,
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
        rs1      = issued.inst.rs1.as_usize(),
        rv1      = %crate::common::trace::Hex(issued.inst.rv1),
        rs2      = issued.inst.rs2.as_usize(),
        rv2      = %crate::common::trace::Hex(issued.inst.rv2),
        imm      = issued.inst.imm,
        alu_op   = ?issued.inst.ctrl.alu,
        "EX: begin"
    );

    if illegal_in_current_state(state.hart(), &issued.inst) {
        return faulted(state, issued, Trap::IllegalInstruction(issued.inst.bits));
    }

    if let Some(executed) = execute_system_inorder(state, issued, rob) {
        return executed;
    }

    if issued.inst.ctrl.vec_op.is_config() {
        let vl = set_vector_config(state, issued, issued.inst.rv1, issued.inst.rv2, rob);
        rob.complete(issued.rob_tag, vl);
        return (ExMem1Entry::from_issue(issued, vl, 0), None);
    }

    // A vector op's registers are read at issue, so what follows one is
    // refetched. Its result is ready when its unit finishes, like any other.
    if issued.inst.ctrl.vec_op != VectorOp::None {
        return match execute_vector(state, issued, rob) {
            Ok(scalar) => (ExMem1Entry::from_issue(issued, scalar, 0), Some(refetch_after(issued))),
            Err(trap) => faulted(state, issued, trap),
        };
    }

    let inst = &issued.inst;
    let (op_a, op_b) = operands(inst);
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
/// MRET, SRET and WFI take effect at commit, which squashes and refetches
/// what follows them, as Rocket's WB does; a CSR access is performed at
/// the head of the ROB, as Rocket's is in WB, so it reads what every older
/// instruction wrote.
fn execute_system_inorder(
    state: &StageCtx<'_>,
    issued: &InOrderEntry,
    rob: &mut Rob,
) -> Option<(ExMem1Entry, Option<Redirect>)> {
    Some(match execute_system(state, issued)? {
        SystemExecution::Done(result, redirect) => (result, redirect),
        SystemExecution::AtRetire => (ExMem1Entry::from_issue(issued, 0, 0), None),
        SystemExecution::Csr(request) => {
            rob.defer_csr(issued.rob_tag, request);
            (ExMem1Entry::from_issue(issued, 0, issued.inst.rv2), None)
        }
    })
}

/// Executes a vector instruction against a shadow of the architectural
/// registers and files what it wrote on its ROB entry for commit.
fn execute_vector(state: &StageCtx<'_>, issued: &InOrderEntry, rob: &mut Rob) -> Result<u64, Trap> {
    let csrs = &state.hart().csrs;
    let vector = &state.config.isa.vector;
    let mut shadow = ShadowVpr::new(state.hart().regs.vpr());
    let mut recorder = AgnosticRecorder::new(&mut shadow, commit_log_open(state));
    let result = execute_vec_op_on(
        &mut recorder,
        csrs.vector_config(),
        csrs.vector_rounding(),
        vector.elen,
        state.core().isa.extensions.contains(Extension::Zvfh),
        &issued.inst,
    )?;
    #[cfg(feature = "commit-log")]
    if let Some(fills) = recorder.into_fills() {
        rob.set_vec_agnostic(issued.rob_tag, fills);
    }
    rob.set_vec_writes(issued.rob_tag, shadow.into_writes());
    record_vector_flags(rob, issued.rob_tag, &result);
    Ok(result.scalar_result.unwrap_or(0))
}

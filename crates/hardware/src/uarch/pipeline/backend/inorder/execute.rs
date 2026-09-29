//! In-Order Execute Unit: single ALU/FPU/BRU execution.
//!
//! This stage performs arithmetic, branch resolution, and system instruction
//! handling. CSR writes and MRET/SRET are deferred to commit via the ROB.

use crate::exec::compute::vector::execute::execute_vec_op_on;
use crate::exec::compute::vector::shadow::ShadowVpr;
use crate::exec::execute::{SystemEffect, evaluate, operands, system_effect, unit_disabled};
use crate::isa::op::VectorOp;
use crate::isa::privileged::Trap;
use crate::system::StageCtx;
use crate::trace_execute;
use crate::uarch::pipeline::backend::shared::execute::{
    fault, propagate_trap, resolve_control_flow,
};
use crate::uarch::pipeline::backend::shared::vector_config::set_vector_config;
use crate::uarch::pipeline::exception::ExceptionStage;
use crate::uarch::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::uarch::pipeline::rob::{Rob, RobTag};
use crate::uarch::pipeline::squash::{Redirect, SquashCause};

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
    entries: &[RenameIssueEntry],
    rob: &mut Rob,
) -> ExecutedBatch {
    let mut batch =
        ExecutedBatch { results: Vec::with_capacity(entries.len()), redirects: Vec::new() };
    for id in entries {
        let (result, redirect) = execute_one(state, id, rob);
        batch.results.push(result);
        batch.redirects.extend(redirect.map(|r| (id.rob_tag, r)));
    }
    batch
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

fn execute_one(
    state: &mut StageCtx<'_>,
    id: &RenameIssueEntry,
    rob: &mut Rob,
) -> (ExMem1Entry, Option<Redirect>) {
    if let Some(trap) = id.trap.clone() {
        return (propagate_trap(state, id, trap), None);
    }

    trace_execute!(state.config.general.trace_instructions;
        rob_tag  = id.rob_tag.0,
        pc       = %crate::sim::trace::Hex(id.inst.pc),
        inst     = %crate::sim::trace::Hex32(id.inst.bits),
        rd       = id.inst.rd.as_usize(),
        rs1      = id.inst.rs1.as_usize(),
        rv1      = %crate::sim::trace::Hex(id.inst.rv1),
        rs2      = id.inst.rs2.as_usize(),
        rv2      = %crate::sim::trace::Hex(id.inst.rv2),
        imm      = id.inst.imm,
        alu_op   = ?id.inst.ctrl.alu,
        "EX: begin"
    );

    if state.check_execute_trigger(id.inst.pc) {
        return faulted(state, id, Trap::Breakpoint(id.inst.pc));
    }

    if let Some(executed) = execute_system(state, id, rob) {
        return executed;
    }

    if unit_disabled(state.hart(), &id.inst) {
        return faulted(state, id, Trap::IllegalInstruction(id.inst.bits));
    }

    if id.inst.ctrl.vec_op.is_config() {
        let vl = set_vector_config(state, id, id.inst.rv1, id.inst.rv2, rob);
        rob.complete(id.rob_tag, vl);
        return (ExMem1Entry::from_issue(id, vl, 0), None);
    }

    // A vector op's registers are read at issue, so what follows one is
    // refetched. Its result is ready when its unit finishes, like any other.
    if id.inst.ctrl.vec_op != VectorOp::None {
        return match execute_vector(state, id, rob) {
            Ok(scalar) => (ExMem1Entry::from_issue(id, scalar, 0), Some(refetch_after(id))),
            Err(trap) => faulted(state, id, trap),
        };
    }

    let inst = &id.inst;
    let (op_a, op_b) = operands(inst);
    let (alu_out, fp_flags) = evaluate(state, inst, op_a, op_b);
    let redirect = match resolve_control_flow(state, rob, id, op_a, op_b) {
        Ok(redirect) => redirect,
        Err(trap) => return faulted(state, id, trap),
    };
    (ExMem1Entry { fp_flags, ..ExMem1Entry::from_issue(id, alu_out, id.inst.rv2) }, redirect)
}

/// Executes a system instruction; `None` for everything else, FENCE
/// included (it orders memory in issue and at commit, not here).
fn execute_system(
    state: &StageCtx<'_>,
    id: &RenameIssueEntry,
    rob: &mut Rob,
) -> Option<(ExMem1Entry, Option<Redirect>)> {
    Some(match system_effect(state, &id.inst) {
        SystemEffect::NotSystem => return None,
        SystemEffect::Trap(trap) => faulted(state, id, trap),
        // FENCE.I's I-cache flush waits for commit, so older stores are
        // visible before the refill.
        SystemEffect::AtRetire => (ExMem1Entry::from_issue(id, 0, 0), Some(refetch_after(id))),
        // The TLB flush waits for commit, after the store buffer drains.
        SystemEffect::SfenceVma(sfence_vma) => {
            let result = ExMem1Entry {
                sfence_vma: Some(sfence_vma),
                ..ExMem1Entry::from_issue(id, 0, id.inst.rv2)
            };
            (result, Some(refetch_after(id)))
        }
        // A CBO passes its operand to memory1, which translates the block;
        // commit performs it. Younger loads wait for it in issue.
        SystemEffect::Cbo(_) => (ExMem1Entry::from_issue(id, id.inst.rv1, 0), None),
        SystemEffect::Csr(access) => {
            if let Some(update) = access.update {
                rob.set_csr_update(id.rob_tag, update.into());
            }
            (ExMem1Entry::from_issue(id, access.old, id.inst.rv2), Some(refetch_after(id)))
        }
    })
}

/// Executes a vector instruction against a shadow of the architectural
/// registers and files what it wrote on its ROB entry for commit.
fn execute_vector(state: &StageCtx<'_>, id: &RenameIssueEntry, rob: &mut Rob) -> Result<u64, Trap> {
    let csrs = &state.hart().csrs;
    let vector = &state.config.isa.vector;
    let mut shadow = ShadowVpr::new(state.hart().regs.vpr());
    let result = execute_vec_op_on(
        &mut shadow,
        csrs.vtype,
        csrs.vl,
        csrs.vstart,
        csrs.vxrm,
        csrs.frm,
        vector.elen,
        vector.zvfh,
        &id.inst,
    )?;
    rob.set_vec_writes(id.rob_tag, shadow.into_writes());
    if result.fp_flags != 0 {
        rob.set_fp_flags(id.rob_tag, result.fp_flags);
    }
    if result.vxsat {
        rob.set_vxsat(id.rob_tag, true);
    }
    Ok(result.scalar_result)
}

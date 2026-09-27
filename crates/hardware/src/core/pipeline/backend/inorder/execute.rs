//! In-Order Execute Unit: single ALU/FPU/BRU execution.
//!
//! This stage performs arithmetic, branch resolution, and system instruction
//! handling. CSR writes and MRET/SRET are deferred to commit via the ROB.

use crate::common::SfenceVmaInfo;
use crate::common::error::{ExceptionStage, Trap};
use crate::core::pipeline::backend::shared::execute::{
    csr_access, ecall_trap, evaluate, fault, next_pc, operands, privileged_op_fault,
    propagate_trap, resolve_branch, resolve_jump, unit_disabled,
};
use crate::core::pipeline::backend::shared::vector_config::set_vector_config;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::signals::{ControlFlow, SystemOp, VectorOp};
use crate::core::pipeline::squash::{Redirect, SquashCause};
use crate::core::units::vpu::execute::execute_vec_op_on;
use crate::core::units::vpu::shadow::ShadowVpr;
use crate::sim::StageCtx;
use crate::trace_execute;

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
    Redirect::to(next_pc(id), SquashCause::System)
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
        pc       = %crate::trace::Hex(id.pc),
        inst     = %crate::trace::Hex32(id.inst),
        rd       = id.rd.as_usize(),
        rs1      = id.rs1.as_usize(),
        rv1      = %crate::trace::Hex(id.rv1),
        rs2      = id.rs2.as_usize(),
        rv2      = %crate::trace::Hex(id.rv2),
        imm      = id.imm,
        alu_op   = ?id.ctrl.alu,
        "EX: begin"
    );

    if state.check_execute_trigger(id.pc) {
        return faulted(state, id, Trap::Breakpoint(id.pc));
    }

    if let Some(executed) = execute_system(state, id, rob) {
        return executed;
    }

    if unit_disabled(state, id) {
        return faulted(state, id, Trap::IllegalInstruction(id.inst));
    }

    if id.ctrl.vec_op.is_config() {
        let vl = set_vector_config(state, id, id.rv1, id.rv2, rob);
        rob.complete(id.rob_tag, vl);
        return (ExMem1Entry::from_issue(id, vl, 0), None);
    }

    // A vector op's registers are read at issue, so what follows one is
    // refetched. Its result is ready when its unit finishes, like any other.
    if id.ctrl.vec_op != VectorOp::None {
        return match execute_vector(state, id, rob) {
            Ok(scalar) => (ExMem1Entry::from_issue(id, scalar, 0), Some(refetch_after(id))),
            Err(trap) => faulted(state, id, trap),
        };
    }

    let (op_a, op_b) = operands(id);
    let (alu_out, fp_flags) = evaluate(state, id, op_a, op_b);
    let redirect = match id.ctrl.control_flow {
        ControlFlow::Branch => resolve_branch(state, rob, id, op_a, op_b),
        ControlFlow::Jump => resolve_jump(state, rob, id),
        ControlFlow::Sequential => None,
    };
    (ExMem1Entry { fp_flags, ..ExMem1Entry::from_issue(id, alu_out, id.rv2) }, redirect)
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
    match id.ctrl.system_op {
        SystemOp::None | SystemOp::Fence => None,
        // FENCE.I's I-cache flush waits for commit, so older stores are
        // visible before the refill.
        SystemOp::FenceI | SystemOp::Mret | SystemOp::Sret | SystemOp::Wfi => {
            Some((ExMem1Entry::from_issue(id, 0, 0), Some(refetch_after(id))))
        }
        // The TLB flush waits for commit, after the store buffer drains.
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
            Some((result, Some(refetch_after(id))))
        }
        // CBO ops gate, translate and take effect at commit, which reads the
        // block address from `alu`; younger loads wait for them in issue.
        SystemOp::CboZero | SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush => {
            Some((ExMem1Entry::from_issue(id, id.rv1, 0), None))
        }
        SystemOp::Ecall => Some(faulted(state, id, ecall_trap(state))),
        SystemOp::Csr => Some(match csr_access(state, id) {
            Ok(access) => {
                if let Some(update) = access.update {
                    rob.set_csr_update(id.rob_tag, update);
                }
                (ExMem1Entry::from_issue(id, access.old, id.rv2), Some(refetch_after(id)))
            }
            Err(trap) => faulted(state, id, trap),
        }),
    }
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
        id,
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

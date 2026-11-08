//! Per-instruction statistics counted at commit.

use crate::exec::signals::ControlFlow;
use crate::isa::op::{AluOpGroup, SystemOp, VecOpGroup, VectorOp};
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::rob::RobEntry;

/// Updates instruction statistics based on the committed entry.
pub(super) fn update_instruction_stats(state: &mut CoreCtx<'_>, entry: &RobEntry) {
    // Check vec ops first: vec loads/stores also set mem_read/mem_write.
    if let Some(op) = entry.ctrl.vec_op {
        update_vec_instruction_stats(state, op);
        return;
    }

    if entry.ctrl.atomic_op.is_some() {
        state.uncore.kernel.stats.counter(state.core.stat_paths.commit.op_atomic).inc();
    } else if entry.ctrl.mem_read {
        if entry.ctrl.fp_reg_write {
            state.uncore.kernel.stats.counter(state.core.stat_paths.commit.fp_load).inc();
        } else {
            state.uncore.kernel.stats.counter(state.core.stat_paths.commit.op_load).inc();
        }
    } else if entry.ctrl.mem_write {
        if entry.ctrl.rs2_fp {
            state.uncore.kernel.stats.counter(state.core.stat_paths.commit.fp_store).inc();
        } else {
            state.uncore.kernel.stats.counter(state.core.stat_paths.commit.op_store).inc();
        }
    } else if matches!(entry.ctrl.control_flow, ControlFlow::Branch(_) | ControlFlow::Jump(_)) {
        state.uncore.kernel.stats.counter(state.core.stat_paths.commit.op_branch).inc();
    } else if !matches!(entry.ctrl.system_op, SystemOp::None) {
        state.uncore.kernel.stats.counter(state.core.stat_paths.commit.op_system).inc();
    } else {
        let paths = &state.core.stat_paths.commit;
        let counter = match entry.ctrl.alu.group() {
            AluOpGroup::FpArith | AluOpGroup::FpMul => paths.fp_arith,
            AluOpGroup::FpDivSqrt => paths.fp_div_sqrt,
            AluOpGroup::FpFma => paths.fp_fma,
            AluOpGroup::IntAlu | AluOpGroup::IntMul | AluOpGroup::IntDiv => paths.op_alu,
        };
        state.uncore.kernel.stats.counter(counter).inc();
    }
}

/// Categorize a vector instruction into the appropriate stat counter.
pub(super) fn update_vec_instruction_stats(state: &mut CoreCtx<'_>, op: VectorOp) {
    let paths = &state.core.stat_paths.commit;
    let counter = match op.group() {
        VecOpGroup::Load => paths.vec_load,
        VecOpGroup::Store => paths.vec_store,
        VecOpGroup::IntAlu | VecOpGroup::IntMul | VecOpGroup::IntDiv => paths.vec_int,
        VecOpGroup::FpAlu | VecOpGroup::FpFma | VecOpGroup::FpDivSqrt | VecOpGroup::FpSlide => {
            paths.vec_fp
        }
        VecOpGroup::Config | VecOpGroup::Permute => paths.vec_misc,
        VecOpGroup::Crypto => paths.vec_crypto,
    };
    state.uncore.kernel.stats.counter(counter).inc();
}

//! The memory-ordering rule both backends' issue stages apply.

use crate::exec::inst::Inst;
use crate::isa::op::SystemOp;
use crate::uarch::pipeline::rob::{Rob, RobTag};

/// Whether a FENCE holds back instruction `tag`: it is a FENCE whose
/// predecessor accesses are still in flight, or an older FENCE or
/// acquiring atomic orders an access it makes.
pub fn held_by_fence(rob: &Rob, tag: RobTag, inst: &Inst) -> bool {
    let unfinished_predecessors = matches!(inst.ctrl.system_op,
        SystemOp::Fence(fence) if !rob.fence_pred_satisfied(tag, fence.pred));
    unfinished_predecessors || rob.has_fence_blocking(tag, inst.ctrl.memory_accesses())
}

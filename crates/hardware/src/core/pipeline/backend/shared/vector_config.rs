//! `vsetvl` execution shared by the backends.

use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::signals::VectorOp;
use crate::core::units::vpu::types::VectorConfig;
use crate::core::units::vpu::vsetvl::execute_vsetvl;
use crate::isa::rvv::encoding as v_enc;
use crate::sim::CoreCtx;

/// Executes a `vsetvl`: records the configuration it establishes on its ROB
/// entry, where younger instructions read it from now and commit writes it
/// to the CSRs. Returns the new `vl`.
pub fn set_vector_config(
    state: &CoreCtx<'_>,
    id: &RenameIssueEntry,
    rs1_value: u64,
    rs2_value: u64,
    rob: &mut Rob,
) -> u64 {
    let (avl, requested_vtype, rs1_is_zero) = match id.ctrl.vec_op {
        VectorOp::Vsetvli => (rs1_value, v_enc::zimm_vsetvli(id.inst), id.rs1.is_zero()),
        VectorOp::Vsetivli => (v_enc::uimm_vsetivli(id.inst), v_enc::zimm_vsetivli(id.inst), false),
        _ => (rs1_value, rs2_value, id.rs1.is_zero()),
    };
    let vlen = state.hart.regs.vpr().vlen();
    let current_vl = rob.youngest_vec_csr_update().map_or(state.hart.csrs.vl, |config| config.vl);
    let (vl, vtype) =
        execute_vsetvl(avl, requested_vtype, id.rd.is_zero(), rs1_is_zero, vlen, current_vl);
    rob.set_vec_csr_update(id.rob_tag, VectorConfig { vtype, vl, vstart: 0 });
    vl
}

//! What vector configuration instructions establish.

use crate::core::exec::inst::Inst;
use crate::core::exec::signals::VectorOp;
use crate::core::units::vpu::types::{VectorConfig, Vlen};
use crate::core::units::vpu::vsetvl::execute_vsetvl;
use crate::isa::rvv::encoding as v_enc;

/// The configuration the `vsetvl` `inst` establishes when `vl` is currently
/// `current_vl`.
#[must_use]
pub fn vector_config(
    inst: &Inst,
    rs1_value: u64,
    rs2_value: u64,
    vlen: Vlen,
    current_vl: u64,
) -> VectorConfig {
    let (avl, requested_vtype, rs1_is_zero) = match inst.ctrl.vec_op {
        VectorOp::Vsetvli => (rs1_value, v_enc::zimm_vsetvli(inst.bits), inst.rs1.is_zero()),
        VectorOp::Vsetivli => {
            (v_enc::uimm_vsetivli(inst.bits), v_enc::zimm_vsetivli(inst.bits), false)
        }
        _ => (rs1_value, rs2_value, inst.rs1.is_zero()),
    };
    let (vl, vtype) =
        execute_vsetvl(avl, requested_vtype, inst.rd.is_zero(), rs1_is_zero, vlen, current_vl);
    VectorConfig { vtype, vl, vstart: 0 }
}

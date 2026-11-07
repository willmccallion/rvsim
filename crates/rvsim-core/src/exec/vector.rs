//! What vector configuration instructions establish.

use crate::exec::compute::vector::vsetvl::execute_vsetvl;
use crate::exec::inst::Inst;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{VectorConfig, Vlen};

/// The configuration the `vsetvl` `inst` establishes when `vl` is currently
/// `current_vl`.
#[must_use]
pub fn vector_config(
    inst: &Inst,
    rs1_value: u64,
    rs2_value: u64,
    vlen: Vlen,
    elen: usize,
    current_vl: u64,
) -> VectorConfig {
    let (avl, requested_vtype, rs1_is_zero) = match inst.ctrl.vec_op {
        VectorOp::Vsetvli { vtype } => (rs1_value, u64::from(vtype), inst.rs1.is_zero()),
        VectorOp::Vsetivli { avl, vtype } => (u64::from(avl), u64::from(vtype), false),
        _ => (rs1_value, rs2_value, inst.rs1.is_zero()),
    };
    let (vl, vtype) = execute_vsetvl(
        avl,
        requested_vtype,
        inst.rd.is_zero(),
        rs1_is_zero,
        vlen,
        elen,
        current_vl,
    );
    VectorConfig { vtype, vl, vstart: 0 }
}

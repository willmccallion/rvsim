//! Vector Integer ALU.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) integer arithmetic
//! operations. The main entry point [`vec_execute`] dispatches on the
//! [`VecAluOp`] the instruction decoded to; each per-element loop handles
//! masking, prestart/tail policy, and the arithmetic itself.

mod compare;
mod integer;
mod widen;

use crate::exec::compute::vector::alu::compare::{exec_carry, exec_comparison};
use crate::exec::compute::vector::alu::integer::{
    exec_extension, exec_macc, exec_merge, exec_standard,
};
use crate::exec::compute::vector::alu::widen::{exec_narrowing, exec_widening, exec_widening_macc};
use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::VecAluOp;
use crate::isa::rvv::{Sew, VRegIdx};

/// Fractional SEW for extension operations (divide by given factor).
#[inline]
const fn frac_sew(sew: Sew, factor: usize) -> Option<Sew> {
    let target = sew.bits() / factor;
    match target {
        8 => Some(Sew::E8),
        16 => Some(Sew::E16),
        32 => Some(Sew::E32),
        64 => Some(Sew::E64),
        _ => None,
    }
}

/// Execute a vector integer ALU operation.
///
/// Iterates over all elements up to VLMAX, applying prestart/tail/mask
/// policies and computing the result for each active element. The destination
/// register group (`vd_idx`) is written in place.
///
#[must_use]
pub fn vec_execute(
    op: VecAluOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        VecAluOp::Int(op) => exec_standard(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::Compare(op) => exec_comparison(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::Carry(op) => exec_carry(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::Macc(op) => exec_macc(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::Widen(op) => exec_widening(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::WidenMacc(op) => exec_widening_macc(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::Narrow(op) => exec_narrowing(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecAluOp::Extend(op) => exec_extension(op, vpr, vd_idx, vs2_idx, ctx),
        VecAluOp::Merge => exec_merge(vpr, vd_idx, vs2_idx, operand1, ctx),
    }
}

#[cfg(test)]
mod tests;

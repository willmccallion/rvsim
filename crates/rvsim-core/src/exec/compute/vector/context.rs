//! The operands, execution context and result every vector unit shares.

use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::RoundingMode;
use crate::isa::rvv::{ElemIdx, MaskPolicy, Sew, TailPolicy, VRegIdx, Vlmul, Vxrm};

/// Source for the first vector operand.
#[derive(Debug, Clone, Copy)]
pub enum VecOperand {
    /// vs1 register index (vector-vector).
    Vector(VRegIdx),
    /// Scalar value from rs1 (vector-scalar).
    Scalar(u64),
    /// Sign-extended 5-bit immediate (vector-immediate).
    Immediate(i64),
}

/// What executing a vector instruction produced besides its register writes.
#[derive(Debug, Default)]
pub struct VecExecResult {
    /// Fixed-point saturation flag (OR of all element saturations).
    pub vxsat: bool,
    /// Scalar result for instructions that write `rd` (`vmv.x.s`, `vcpop.m`,
    /// `vfirst.m`).
    pub scalar_result: Option<u64>,
    /// Accumulated floating-point exception flags (OR of all elements).
    pub fp_flags: crate::isa::fp::FpFlags,
}

/// Context bundle for vector execution loops.
///
/// Groups the common parameters that every execution loop needs, reducing
/// the argument count of internal dispatch functions.
#[derive(Debug)]
pub struct VecExecCtx {
    /// Selected element width.
    pub sew: Sew,
    /// Current vector length.
    pub vl: usize,
    /// Elements before this index are prestart (skipped).
    pub vstart: usize,
    /// Masked-off element policy.
    pub vma: MaskPolicy,
    /// Tail element policy.
    pub vta: TailPolicy,
    /// Vector length multiplier.
    pub vlmul: Vlmul,
    /// Masking mode: `true` = unmasked, `false` = masked by v0.
    pub vm: bool,
    /// Fixed-point rounding mode.
    pub vxrm: Vxrm,
    /// FP rounding mode from `fcsr.frm` (used by vector FP operations).
    pub frm: RoundingMode,
    /// Whether the Zvfh (half-precision vector FP) extension is enabled.
    pub zvfh: bool,
}

/// Read v0 mask bit for element `i`.
#[inline]
pub(super) fn mask_active(vpr: &impl VectorRegFile, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Read operand1 value for element `i` at the given SEW, applying the mask.
#[inline]
pub(super) fn read_op1(vpr: &impl VectorRegFile, operand1: &VecOperand, i: usize, sew: Sew) -> u64 {
    match operand1 {
        VecOperand::Vector(vs1) => vpr.read_element(*vs1, ElemIdx::new(i), sew),
        VecOperand::Scalar(s) => *s & sew.mask(),
        VecOperand::Immediate(imm) => (*imm as u64) & sew.mask(),
    }
}

/// Sign-extend a SEW-width value stored in a `u64` to a full `i64`.
#[inline]
pub(super) const fn sign_extend(val: u64, sew: Sew) -> i64 {
    let shift = 64 - sew.bits();
    ((val << shift) as i64) >> shift
}

/// Widen a SEW to the next larger width. Returns `None` for E64.
#[inline]
pub(super) const fn widen_sew(sew: Sew) -> Option<Sew> {
    match sew {
        Sew::E8 => Some(Sew::E16),
        Sew::E16 => Some(Sew::E32),
        Sew::E32 => Some(Sew::E64),
        Sew::E64 => None,
    }
}

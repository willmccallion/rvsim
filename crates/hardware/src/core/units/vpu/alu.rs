//! Vector Integer ALU.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) integer arithmetic
//! operations. The main entry point [`vec_execute`] dispatches to per-element
//! loops that handle masking, prestart/tail policy, and the arithmetic itself.
//!
//! Operations are grouped into categories:
//! - Standard arithmetic: add, sub, rsub, and, or, xor, shifts, min/max
//! - Comparisons (write mask): seq, sne, slt, sle, sgt, etc.
//! - Add/subtract with carry: adc, sbc, madc, msbc
//! - Multiply / multiply-accumulate
//! - Division / remainder
//! - Widening arithmetic and multiply
//! - Narrowing shifts and clips
//! - Saturating and averaging arithmetic
//! - Fixed-point scaling: smul, ssrl, ssra
//! - Extension: zero/sign-extend at various ratios

use crate::core::arch::vpr::Vpr;
use crate::core::pipeline::signals::VectorOp;
use crate::core::units::vpu::types::{
    ElemIdx, MaskPolicy, Sew, TailPolicy, VRegIdx, Vlmax, Vlmul, Vxrm,
};

// ============================================================================
// Public types
// ============================================================================

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

/// Result of a vector ALU operation.
#[derive(Debug)]
pub struct VecExecResult {
    /// Fixed-point saturation flag (OR of all element saturations).
    pub vxsat: bool,
    /// Scalar result for instructions that write rd (reserved for future use).
    pub scalar_result: Option<u64>,
    /// Accumulated floating-point exception flags (OR of all elements).
    pub fp_flags: crate::core::units::fpu::exception_flags::FpFlags,
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
}

// ============================================================================
// Helpers
// ============================================================================

/// Sign-extend a SEW-width value stored in a `u64` to a full `i64`.
#[inline]
const fn sign_extend(val: u64, sew: Sew) -> i64 {
    let shift = 64 - sew.bits();
    ((val << shift) as i64) >> shift
}

/// Widen a SEW to the next larger width. Returns `None` for E64.
#[inline]
const fn widen_sew(sew: Sew) -> Option<Sew> {
    match sew {
        Sew::E8 => Some(Sew::E16),
        Sew::E16 => Some(Sew::E32),
        Sew::E32 => Some(Sew::E64),
        Sew::E64 => None,
    }
}

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

/// Read v0 mask bit for element `i`.
#[inline]
fn mask_active(vpr: &Vpr, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Write all-1s at the given SEW width to a destination element.
#[inline]
fn write_ones(vpr: &mut Vpr, vd: VRegIdx, i: usize, sew: Sew) {
    vpr.write_element(vd, ElemIdx::new(i), sew, sew.mask());
}

/// Read operand1 value for element `i` at the given SEW, applying the mask.
#[inline]
fn read_op1(vpr: &Vpr, operand1: &VecOperand, i: usize, sew: Sew) -> u64 {
    match operand1 {
        VecOperand::Vector(vs1) => vpr.read_element(*vs1, ElemIdx::new(i), sew),
        VecOperand::Scalar(s) => *s & sew.mask(),
        VecOperand::Immediate(imm) => (*imm as u64) & sew.mask(),
    }
}

/// Compute the fixed-point rounding increment for averaging / scaling ops.
///
/// Per the RVV spec, the rounding bit `r` depends on `vxrm` and the value
/// being shifted. `v` is the full pre-shift value, `d` is the shift amount.
#[inline]
const fn rounding_incr(v: u64, d: u32, vxrm: Vxrm) -> u64 {
    if d == 0 {
        return 0;
    }
    match vxrm {
        Vxrm::RoundToNearestUp => (v >> (d - 1)) & 1,
        Vxrm::RoundToNearestEven => {
            let r = (v >> (d - 1)) & 1;
            // "sticky" = OR of bits below the rounding position
            let sticky = if d >= 2 { v & ((1u64 << (d - 1)) - 1) } else { 0 };
            // round to nearest even: r & (sticky | lsb_of_result)
            let lsb = (v >> d) & 1;
            r & (sticky | lsb)
        }
        Vxrm::RoundDown => 0,
        Vxrm::RoundToOdd => {
            let dropped = v & ((1u64 << d) - 1);
            if dropped != 0 { 1 } else { 0 }
        }
    }
}

// ============================================================================
// Standard element computation
// ============================================================================

/// Compute one element for standard (non-widening, non-narrowing) integer ops.
#[inline]
fn compute_standard(op: VectorOp, vs2: u64, op1: u64, sew: Sew, vxrm: Vxrm) -> (u64, bool) {
    let mask = sew.mask();
    let bits = sew.bits();
    let s2 = sign_extend(vs2, sew);
    let s1 = sign_extend(op1, sew);

    match op {
        // ── Arithmetic ──────────────────────────────────────────────────
        VectorOp::VAdd => (vs2.wrapping_add(op1) & mask, false),
        VectorOp::VSub => (vs2.wrapping_sub(op1) & mask, false),
        VectorOp::VRsub => (op1.wrapping_sub(vs2) & mask, false),

        // ── Bitwise ─────────────────────────────────────────────────────
        VectorOp::VAnd => (vs2 & op1, false),
        VectorOp::VOr => (vs2 | op1, false),
        VectorOp::VXor => (vs2 ^ op1, false),

        // ── Shifts ──────────────────────────────────────────────────────
        VectorOp::VSll => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            ((vs2 << shamt) & mask, false)
        }
        VectorOp::VSrl => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            ((vs2 >> shamt) & mask, false)
        }
        VectorOp::VSra => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let result = (s2 >> shamt) as u64;
            (result & mask, false)
        }

        // ── Min / Max ───────────────────────────────────────────────────
        VectorOp::VMin => ((if s2 < s1 { vs2 } else { op1 }) & mask, false),
        VectorOp::VMinU => (if vs2 < op1 { vs2 } else { op1 }, false),
        VectorOp::VMax => ((if s2 > s1 { vs2 } else { op1 }) & mask, false),
        VectorOp::VMaxU => (if vs2 > op1 { vs2 } else { op1 }, false),

        // ── Multiply low ────────────────────────────────────────────────
        VectorOp::VMul => (vs2.wrapping_mul(op1) & mask, false),

        // ── Multiply high ───────────────────────────────────────────────
        VectorOp::VMulh => {
            let prod = (s2 as i128).wrapping_mul(s1 as i128);
            let hi = (prod >> bits) as u64;
            (hi & mask, false)
        }
        VectorOp::VMulhu => {
            let prod = (vs2 as u128).wrapping_mul(op1 as u128);
            let hi = (prod >> bits) as u64;
            (hi & mask, false)
        }
        VectorOp::VMulhsu => {
            let prod = (s2 as i128).wrapping_mul(op1 as i128);
            let hi = (prod >> bits) as u64;
            (hi & mask, false)
        }

        // ── Division ────────────────────────────────────────────────────
        VectorOp::VDivU => {
            if op1 == 0 {
                (mask, false)
            } else {
                ((vs2 / op1) & mask, false)
            }
        }
        VectorOp::VDiv => {
            if op1 == 0 {
                // div by zero: all-1s (which is -1 signed)
                (mask, false)
            } else {
                let min_int = 1u64 << (bits - 1);
                let neg_one = mask; // all 1s at SEW width
                if vs2 == min_int && op1 == neg_one {
                    // signed overflow: MIN_INT / -1 = MIN_INT
                    (min_int & mask, false)
                } else {
                    let result = s2.wrapping_div(s1) as u64;
                    (result & mask, false)
                }
            }
        }
        VectorOp::VRemU => {
            if op1 == 0 {
                (vs2, false)
            } else {
                ((vs2 % op1) & mask, false)
            }
        }
        VectorOp::VRem => {
            if op1 == 0 {
                (vs2, false)
            } else {
                let min_int = 1u64 << (bits - 1);
                let neg_one = mask;
                if vs2 == min_int && op1 == neg_one {
                    // signed overflow: MIN_INT % -1 = 0
                    (0, false)
                } else {
                    let result = s2.wrapping_rem(s1) as u64;
                    (result & mask, false)
                }
            }
        }

        // ── Saturating add/sub ──────────────────────────────────────────
        VectorOp::VSAddU => {
            let sum = vs2.wrapping_add(op1) & mask;
            if sum < vs2 { (mask, true) } else { (sum, false) }
        }
        VectorOp::VSAdd => {
            let max_pos = (1u64 << (bits - 1)) - 1;
            let min_neg = 1u64 << (bits - 1); // as unsigned representation
            let sum = s2.wrapping_add(s1);
            if s1 > 0 && sum < s2 {
                // positive overflow
                (max_pos, true)
            } else if s1 < 0 && sum > s2 {
                // negative overflow
                (min_neg & mask, true)
            } else {
                (sum as u64 & mask, false)
            }
        }
        VectorOp::VSSubU => {
            if vs2 < op1 {
                (0, true)
            } else {
                (vs2.wrapping_sub(op1) & mask, false)
            }
        }
        VectorOp::VSSub => {
            let max_pos = (1u64 << (bits - 1)) - 1;
            let min_neg = 1u64 << (bits - 1);
            let diff = s2.wrapping_sub(s1);
            if s1 < 0 && diff < s2 {
                // positive overflow (subtracting negative overflowed positive)
                (max_pos, true)
            } else if s1 > 0 && diff > s2 {
                // negative overflow
                (min_neg & mask, true)
            } else {
                (diff as u64 & mask, false)
            }
        }

        // ── Averaging add/sub ───────────────────────────────────────────
        VectorOp::VAAddU => {
            let sum = (vs2 as u128) + (op1 as u128);
            let r = rounding_incr(sum as u64, 1, vxrm);
            let result = ((sum >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }
        VectorOp::VAAdd => {
            let sum = (s2 as i128) + (s1 as i128);
            let r = rounding_incr(sum as u64, 1, vxrm);
            let result = ((sum >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }
        VectorOp::VASubU => {
            let diff = (vs2 as i128) - (op1 as i128);
            let r = rounding_incr(diff as u64, 1, vxrm);
            let result = ((diff >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }
        VectorOp::VASub => {
            let diff = (s2 as i128) - (s1 as i128);
            let r = rounding_incr(diff as u64, 1, vxrm);
            let result = ((diff >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }

        // ── Fractional multiply (vsmul) ─────────────────────────────────
        VectorOp::VSmul => {
            let prod = (s2 as i128) * (s1 as i128);
            let shift = bits - 1;
            let r = rounding_incr(prod as u64, shift as u32, vxrm);
            let result_wide = (prod >> shift) + r as i128;
            let max_pos = (1i64 << (bits - 1)) - 1;
            let min_neg = -(1i64 << (bits - 1));
            let sat;
            let clamped = if result_wide > max_pos as i128 {
                sat = true;
                max_pos as u64
            } else if result_wide < min_neg as i128 {
                sat = true;
                min_neg as u64
            } else {
                sat = false;
                result_wide as u64
            };
            (clamped & mask, sat)
        }

        // ── Scaling shifts ──────────────────────────────────────────────
        VectorOp::VSSrl => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let r = rounding_incr(vs2, shamt, vxrm);
            let result = (vs2 >> shamt).wrapping_add(r);
            (result & mask, false)
        }
        VectorOp::VSSra => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let r = rounding_incr(vs2, shamt, vxrm);
            let result = ((s2 >> shamt) as u64).wrapping_add(r);
            (result & mask, false)
        }

        _ => unreachable!(),
    }
}

// ============================================================================
// Comparison helpers
// ============================================================================

/// Evaluate a comparison for one element, returning a bool for the mask bit.
#[inline]
fn compute_compare(op: VectorOp, vs2: u64, op1: u64, sew: Sew) -> bool {
    let s2 = sign_extend(vs2, sew);
    let s1 = sign_extend(op1, sew);
    match op {
        VectorOp::VMSeq => vs2 == op1,
        VectorOp::VMSne => vs2 != op1,
        VectorOp::VMSltu => vs2 < op1,
        VectorOp::VMSlt => s2 < s1,
        VectorOp::VMSleu => vs2 <= op1,
        VectorOp::VMSle => s2 <= s1,
        VectorOp::VMSgtu => vs2 > op1,
        VectorOp::VMSgt => s2 > s1,
        _ => unreachable!(),
    }
}

// ============================================================================
// Widening element computation
// ============================================================================

/// Compute one widening element. Reads sources at `sew`, writes at `wsew`.
/// For `.w` variants, vs2 is already at `wsew`.
#[inline]
fn compute_widening(op: VectorOp, vs2_val: u64, op1_val: u64, sew: Sew, wsew: Sew) -> u64 {
    let wmask = wsew.mask();

    // Sign- and zero-extend narrow operands to the wide width.
    let s2_narrow = sign_extend(vs2_val, sew) as u64 & wmask;
    let u2_narrow = vs2_val & sew.mask();
    let s1 = sign_extend(op1_val, sew) as u64 & wmask;
    let u1 = op1_val & sew.mask();

    // For `.w` variants, vs2 is already wide.
    let s2_wide = sign_extend(vs2_val, wsew) as u64 & wmask;
    let u2_wide = vs2_val & wmask;

    match op {
        VectorOp::VWAddU => u2_narrow.wrapping_add(u1) & wmask,
        VectorOp::VWAdd => s2_narrow.wrapping_add(s1) & wmask,
        VectorOp::VWSubU => u2_narrow.wrapping_sub(u1) & wmask,
        VectorOp::VWSub => s2_narrow.wrapping_sub(s1) & wmask,

        VectorOp::VWAddUW => u2_wide.wrapping_add(u1) & wmask,
        VectorOp::VWAddW => s2_wide.wrapping_add(s1) & wmask,
        VectorOp::VWSubUW => u2_wide.wrapping_sub(u1) & wmask,
        VectorOp::VWSubW => s2_wide.wrapping_sub(s1) & wmask,

        VectorOp::VWMulU => {
            let prod = (u2_narrow as u128) * (u1 as u128);
            prod as u64 & wmask
        }
        VectorOp::VWMul => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (sign_extend(op1_val, sew) as i128);
            prod as u64 & wmask
        }
        VectorOp::VWMulSU => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (u1 as i128);
            prod as u64 & wmask
        }
        _ => unreachable!(),
    }
}

/// Compute one widening multiply-accumulate element.
#[inline]
fn compute_widening_macc(
    op: VectorOp,
    vs2_val: u64,
    op1_val: u64,
    vd_val: u64,
    sew: Sew,
    wsew: Sew,
) -> u64 {
    let wmask = wsew.mask();
    let u2 = vs2_val & sew.mask();
    let u1 = op1_val & sew.mask();
    let acc = vd_val & wmask;

    match op {
        VectorOp::VWMaccU => {
            let prod = (u2 as u128) * (u1 as u128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        VectorOp::VWMacc => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (sign_extend(op1_val, sew) as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        VectorOp::VWMaccSU => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (u1 as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        VectorOp::VWMaccUS => {
            // op1 is signed, vs2 is unsigned
            let prod = (u2 as i128) * (sign_extend(op1_val, sew) as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        _ => unreachable!(),
    }
}

// ============================================================================
// Narrowing element computation
// ============================================================================

/// Compute one narrowing element. Reads vs2 at `wsew` (2*SEW), shift amount
/// from op1 at `sew`, writes result at `sew`.
#[inline]
fn compute_narrowing(
    op: VectorOp,
    vs2_val: u64,
    op1_val: u64,
    sew: Sew,
    wsew: Sew,
    vxrm: Vxrm,
) -> (u64, bool) {
    let mask = sew.mask();
    let wbits = wsew.bits();
    let shamt = (op1_val & (wbits as u64 - 1)) as u32;

    match op {
        VectorOp::VNSrl => {
            let result = vs2_val >> shamt;
            (result & mask, false)
        }
        VectorOp::VNSra => {
            let s = sign_extend(vs2_val, wsew);
            let result = (s >> shamt) as u64;
            (result & mask, false)
        }
        VectorOp::VNClipU => {
            let r = rounding_incr(vs2_val, shamt, vxrm);
            let shifted = (vs2_val >> shamt).wrapping_add(r);
            if shifted > mask { (mask, true) } else { (shifted & mask, false) }
        }
        VectorOp::VNClip => {
            let s = sign_extend(vs2_val, wsew);
            let r = rounding_incr(vs2_val, shamt, vxrm) as i64;
            let shifted = (s >> shamt).wrapping_add(r);
            let max_pos = (1i64 << (sew.bits() - 1)) - 1;
            let min_neg = -(1i64 << (sew.bits() - 1));
            if shifted > max_pos {
                (max_pos as u64 & mask, true)
            } else if shifted < min_neg {
                (min_neg as u64 & mask, true)
            } else {
                (shifted as u64 & mask, false)
            }
        }
        _ => unreachable!(),
    }
}

// ============================================================================
// Category detection helpers
// ============================================================================

#[inline]
const fn is_comparison(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VMSeq
            | VectorOp::VMSne
            | VectorOp::VMSltu
            | VectorOp::VMSlt
            | VectorOp::VMSleu
            | VectorOp::VMSle
            | VectorOp::VMSgtu
            | VectorOp::VMSgt
    )
}

#[inline]
const fn is_carry_op(op: VectorOp) -> bool {
    matches!(op, VectorOp::VAdc | VectorOp::VMadc | VectorOp::VSbc | VectorOp::VMsbc)
}

#[inline]
const fn is_mask_producing_carry(op: VectorOp) -> bool {
    matches!(op, VectorOp::VMadc | VectorOp::VMsbc)
}

#[inline]
const fn is_widening(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VWAddU
            | VectorOp::VWAdd
            | VectorOp::VWSubU
            | VectorOp::VWSub
            | VectorOp::VWAddUW
            | VectorOp::VWAddW
            | VectorOp::VWSubUW
            | VectorOp::VWSubW
            | VectorOp::VWMulU
            | VectorOp::VWMul
            | VectorOp::VWMulSU
    )
}

#[inline]
const fn is_widening_macc(op: VectorOp) -> bool {
    matches!(op, VectorOp::VWMaccU | VectorOp::VWMacc | VectorOp::VWMaccSU | VectorOp::VWMaccUS)
}

/// `.w` variants read vs2 at the wide (2*SEW) width.
#[inline]
const fn is_wide_vs2(op: VectorOp) -> bool {
    matches!(op, VectorOp::VWAddUW | VectorOp::VWAddW | VectorOp::VWSubUW | VectorOp::VWSubW)
}

#[inline]
const fn is_narrowing(op: VectorOp) -> bool {
    matches!(op, VectorOp::VNSrl | VectorOp::VNSra | VectorOp::VNClipU | VectorOp::VNClip)
}

#[inline]
const fn is_macc(op: VectorOp) -> bool {
    matches!(op, VectorOp::VMacc | VectorOp::VNMSac | VectorOp::VMadd | VectorOp::VNMSub)
}

#[inline]
const fn is_extension(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VZextVf2
            | VectorOp::VZextVf4
            | VectorOp::VZextVf8
            | VectorOp::VSextVf2
            | VectorOp::VSextVf4
            | VectorOp::VSextVf8
    )
}

// ============================================================================
// Main entry point
// ============================================================================

/// Execute a vector integer ALU operation.
///
/// Iterates over all elements up to VLMAX, applying prestart/tail/mask
/// policies and computing the result for each active element. The destination
/// register group (`vd_idx`) is written in place.
///
/// # Arguments
///
/// * `op`       - The vector ALU operation to perform.
/// * `vpr`      - Mutable reference to the vector register file.
/// * `vd_idx`   - Destination vector register index.
/// * `vs2_idx`  - Second source vector register index.
/// * `operand1` - First source operand (vector, scalar, or immediate).
/// * `sew`      - Selected element width.
/// * `vl`       - Current vector length.
/// * `vstart`   - Current vstart value (elements before this are prestart).
/// * `vma`      - Masked-off element policy.
/// * `vta`      - Tail element policy.
/// * `vlmul`    - Vector length multiplier.
/// * `vm`       - Masking mode: `true` = unmasked, `false` = masked by v0.
/// * `vxrm`     - Fixed-point rounding mode.
#[allow(clippy::too_many_arguments)]
pub fn vec_execute(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    sew: Sew,
    vl: usize,
    vstart: usize,
    vma: MaskPolicy,
    vta: TailPolicy,
    vlmul: Vlmul,
    vm: bool,
    vxrm: Vxrm,
) -> VecExecResult {
    let ctx = VecExecCtx { sew, vl, vstart, vma, vta, vlmul, vm, vxrm };

    // ── Dispatch by category ────────────────────────────────────────────
    if is_comparison(op) {
        return exec_comparison(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_carry_op(op) {
        return exec_carry(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_widening(op) {
        return exec_widening(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_widening_macc(op) {
        return exec_widening_macc(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_narrowing(op) {
        return exec_narrowing(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_extension(op) {
        return exec_extension(op, vpr, vd_idx, vs2_idx, &ctx);
    }
    if is_macc(op) {
        return exec_macc(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if op == VectorOp::VMerge {
        return exec_merge(vpr, vd_idx, vs2_idx, operand1, &ctx);
    }

    // ── Standard element-wise operations ────────────────────────────────
    exec_standard(op, vpr, vd_idx, vs2_idx, operand1, &ctx)
}

// ============================================================================
// Execution loops
// ============================================================================

/// Standard (non-widening, non-narrowing) element-wise loop.
fn exec_standard(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut vxsat = false;

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let (result, sat) = compute_standard(op, vs2_val, op1_val, ctx.sew, ctx.vxrm);
        vxsat |= sat;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult {
        vxsat,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Comparison loop: writes mask bits to vd.
fn exec_comparison(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let result = compute_compare(op, vs2_val, op1_val, ctx.sew);
        vpr.write_mask_bit(vd_idx, ElemIdx::new(i), result);
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Add/subtract with carry loop.
fn exec_carry(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mask = ctx.sew.mask();
    let writes_mask = is_mask_producing_carry(op);

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                if writes_mask {
                    vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
                } else {
                    write_ones(vpr, vd_idx, i, ctx.sew);
                }
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        // Carry/borrow comes from v0 mask. For vmadc/vmsbc with vm=1,
        // carry is 0 (no carry input).
        let carry = if ctx.vm { 0u64 } else { mask_active(vpr, i) as u64 };

        match op {
            VectorOp::VAdc => {
                let result = vs2_val.wrapping_add(op1_val).wrapping_add(carry) & mask;
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
            }
            VectorOp::VMadc => {
                let sum = (vs2_val as u128) + (op1_val as u128) + (carry as u128);
                let cout = sum > mask as u128;
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), cout);
            }
            VectorOp::VSbc => {
                let result = vs2_val.wrapping_sub(op1_val).wrapping_sub(carry) & mask;
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
            }
            VectorOp::VMsbc => {
                let borrow = (vs2_val as u128) < (op1_val as u128) + (carry as u128);
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), borrow);
            }
            _ => unreachable!(),
        }
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Multiply-accumulate loop (vmacc, vnmsac, vmadd, vnmsub).
fn exec_macc(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mask = ctx.sew.mask();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_val = vpr.read_element(vd_idx, ElemIdx::new(i), ctx.sew);

        let result = match op {
            // vd = vs1 * vs2 + vd
            VectorOp::VMacc => op1_val.wrapping_mul(vs2_val).wrapping_add(vd_val) & mask,
            // vd = -(vs1 * vs2) + vd
            VectorOp::VNMSac => vd_val.wrapping_sub(op1_val.wrapping_mul(vs2_val)) & mask,
            // vd = vs1 * vd + vs2
            VectorOp::VMadd => op1_val.wrapping_mul(vd_val).wrapping_add(vs2_val) & mask,
            // vd = -(vs1 * vd) + vs2
            VectorOp::VNMSub => vs2_val.wrapping_sub(op1_val.wrapping_mul(vd_val)) & mask,
            _ => unreachable!(),
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Merge/move loop.
///
/// `vmerge.vvm vd, vs2, vs1, v0` — masked merge. When `vm=true` this acts
/// as a simple move of operand1 into vd (all elements from op1).
fn exec_merge(
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }

        // merge: if mask bit set (or vm=true for vmv), take op1; else take vs2
        let use_op1 = ctx.vm || mask_active(vpr, i);
        let result = if use_op1 {
            read_op1(vpr, &operand1, i, ctx.sew)
        } else {
            vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew)
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Widening (non-accumulate) loop.
fn exec_widening(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
        };
    };
    // Destination VLMAX is computed at the wider SEW with doubled LMUL.
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let vs2_sew = if is_wide_vs2(op) { wsew } else { ctx.sew };

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, wsew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, wsew);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), vs2_sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let result = compute_widening(op, vs2_val, op1_val, ctx.sew, wsew);
        vpr.write_element(vd_idx, ElemIdx::new(i), wsew, result);
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Widening multiply-accumulate loop.
fn exec_widening_macc(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
        };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, wsew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, wsew);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_val = vpr.read_element(vd_idx, ElemIdx::new(i), wsew);
        let result = compute_widening_macc(op, vs2_val, op1_val, vd_val, ctx.sew, wsew);
        vpr.write_element(vd_idx, ElemIdx::new(i), wsew, result);
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Narrowing loop.
fn exec_narrowing(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
        };
    };
    // sew is the destination width; wsew = 2*sew is the source width.
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut vxsat = false;

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }

        // vs2 is read at the wide (2*SEW) width
        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), wsew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let (result, sat) = compute_narrowing(op, vs2_val, op1_val, ctx.sew, wsew, ctx.vxrm);
        vxsat |= sat;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult {
        vxsat,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

/// Extension loop (vzext, vsext).
fn exec_extension(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    let (factor, is_signed) = match op {
        VectorOp::VZextVf2 => (2, false),
        VectorOp::VZextVf4 => (4, false),
        VectorOp::VZextVf8 => (8, false),
        VectorOp::VSextVf2 => (2, true),
        VectorOp::VSextVf4 => (4, true),
        VectorOp::VSextVf8 => (8, true),
        _ => unreachable!(),
    };

    let Some(src_sew) = frac_sew(ctx.sew, factor) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
        };
    };

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
            continue;
        }

        let src_val = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);
        let result =
            if is_signed { sign_extend(src_val, src_sew) as u64 & ctx.sew.mask() } else { src_val };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult {
        vxsat: false,
        scalar_result: None,
        fp_flags: crate::core::units::fpu::exception_flags::FpFlags::NONE,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::units::vpu::types::Vlen;

    /// Helper: create a 128-bit VLEN VPR.
    fn make_vpr() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    /// Helper: execute with common defaults (LMUL=1, unmasked, vstart=0,
    /// undisturbed policies).
    fn run(
        op: VectorOp,
        vpr: &mut Vpr,
        vd: VRegIdx,
        vs2: VRegIdx,
        operand1: VecOperand,
        sew: Sew,
        vl: usize,
    ) -> VecExecResult {
        vec_execute(
            op,
            vpr,
            vd,
            vs2,
            operand1,
            sew,
            vl,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        )
    }

    // ── Basic add at all SEW widths ─────────────────────────────────────

    #[test]
    fn test_vadd_e8() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        // Write 100 to vs2[0], operate with scalar 55
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 100);
        let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(55), Sew::E8, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 155);
    }

    #[test]
    fn test_vadd_e16() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 1000);
        let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(2345), Sew::E16, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 3345);
    }

    #[test]
    fn test_vadd_e32() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0x8000_0000);
        let _ =
            run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(0x8000_0000), Sew::E32, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0);
    }

    #[test]
    fn test_vadd_e64() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E64, 0xFFFF_FFFF_FFFF_FFFE);
        let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(3), Sew::E64, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E64), 1);
    }

    // ── Vector-vector add ───────────────────────────────────────────────

    #[test]
    fn test_vadd_vv_multiple_elements() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(3);
        let vs2 = VRegIdx::new(4);
        let vs1 = VRegIdx::new(5);
        // VLEN=128, SEW=32 → 4 elements per register
        for i in 0..4 {
            vpr.write_element(vs2, ElemIdx::new(i), Sew::E32, (i as u64) * 10);
            vpr.write_element(vs1, ElemIdx::new(i), Sew::E32, (i as u64) + 1);
        }
        let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Vector(vs1), Sew::E32, 4);
        for i in 0..4 {
            let expected = (i as u64) * 10 + (i as u64) + 1;
            assert_eq!(vpr.read_element(vd, ElemIdx::new(i), Sew::E32), expected);
        }
    }

    // ── Signed comparison ───────────────────────────────────────────────

    #[test]
    fn test_vmslt_signed() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        // Write -1 (0xFF) to vs2[0] at E8, compare with 1
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
        let _ = run(VectorOp::VMSlt, &mut vpr, vd, vs2, VecOperand::Scalar(1), Sew::E8, 1);
        // -1 < 1 should be true
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
    }

    #[test]
    fn test_vmseq() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 42);
        vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, 43);
        let _ = run(VectorOp::VMSeq, &mut vpr, vd, vs2, VecOperand::Scalar(42), Sew::E32, 2);
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(1)));
    }

    // ── Division by zero ────────────────────────────────────────────────

    #[test]
    fn test_vdivu_by_zero() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 42);
        let _ = run(VectorOp::VDivU, &mut vpr, vd, vs2, VecOperand::Scalar(0), Sew::E32, 1);
        // div by zero → all-1s
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0xFFFF_FFFF);
    }

    #[test]
    fn test_vdiv_by_zero() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 100);
        let _ = run(VectorOp::VDiv, &mut vpr, vd, vs2, VecOperand::Scalar(0), Sew::E16, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0xFFFF);
    }

    #[test]
    fn test_vremu_by_zero() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 42);
        let _ = run(VectorOp::VRemU, &mut vpr, vd, vs2, VecOperand::Scalar(0), Sew::E32, 1);
        // rem by zero → dividend
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 42);
    }

    #[test]
    fn test_vdiv_signed_overflow() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        // MIN_INT(E32) = 0x80000000, -1 = 0xFFFFFFFF
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0x8000_0000);
        let _ =
            run(VectorOp::VDiv, &mut vpr, vd, vs2, VecOperand::Scalar(0xFFFF_FFFF), Sew::E32, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0x8000_0000);
    }

    // ── Widening add ────────────────────────────────────────────────────

    #[test]
    fn test_vwaddu() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(4);
        // E16 + E16 → E32
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 0xFFFF);
        let res = vec_execute(
            VectorOp::VWAddU,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(1),
            Sew::E16,
            1,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        );
        assert!(!res.vxsat);
        // 0xFFFF + 1 = 0x10000 (doesn't overflow because result is E32)
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0x10000);
    }

    #[test]
    fn test_vwadd_signed() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(4);
        // -1 at E8 (0xFF) + -2 at E8 (0xFE) → -3 at E16 (0xFFFD)
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
        let _ = vec_execute(
            VectorOp::VWAdd,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(0xFE),
            Sew::E8,
            1,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        );
        let result = vpr.read_element(vd, ElemIdx::new(0), Sew::E16);
        // sign_extend(0xFF, E8) = -1, sign_extend(0xFE, E8) = -2, sum = -3
        // -3 as u16 = 0xFFFD
        assert_eq!(result, 0xFFFD);
    }

    // ── Saturating add ──────────────────────────────────────────────────

    #[test]
    fn test_vsaddu_saturation() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 200);
        let res = run(VectorOp::VSAddU, &mut vpr, vd, vs2, VecOperand::Scalar(100), Sew::E8, 1);
        // 200 + 100 = 300, saturates to 255
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 0xFF);
        assert!(res.vxsat);
    }

    #[test]
    fn test_vsaddu_no_saturation() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 100);
        let res = run(VectorOp::VSAddU, &mut vpr, vd, vs2, VecOperand::Scalar(50), Sew::E8, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 150);
        assert!(!res.vxsat);
    }

    // ── Masking ─────────────────────────────────────────────────────────

    #[test]
    fn test_masked_operation() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let v0 = VRegIdx::new(0);

        // Set up mask: element 0 active, element 1 inactive
        vpr.write_mask_bit(v0, ElemIdx::new(0), true);
        vpr.write_mask_bit(v0, ElemIdx::new(1), false);

        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 10);
        vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, 20);

        // Pre-fill vd with sentinel values
        vpr.write_element(vd, ElemIdx::new(0), Sew::E32, 0xDEAD);
        vpr.write_element(vd, ElemIdx::new(1), Sew::E32, 0xBEEF);

        let _ = vec_execute(
            VectorOp::VAdd,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(5),
            Sew::E32,
            2,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            false, // masked
            Vxrm::RoundToNearestUp,
        );

        // Element 0 is active: 10 + 5 = 15
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 15);
        // Element 1 is inactive with undisturbed policy: keep 0xBEEF
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 0xBEEF);
    }

    // ── Tail handling ───────────────────────────────────────────────────

    #[test]
    fn test_tail_agnostic() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);

        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 10);
        // Pre-fill tail element
        vpr.write_element(vd, ElemIdx::new(1), Sew::E32, 0x1234);

        let _ = vec_execute(
            VectorOp::VAdd,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(5),
            Sew::E32,
            1, // vl=1, so element 1 is tail
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Agnostic,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        );

        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 15);
        // Tail element with agnostic: all-1s
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 0xFFFF_FFFF);
    }

    // ── Merge ───────────────────────────────────────────────────────────

    #[test]
    fn test_vmerge() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(3);
        let vs2 = VRegIdx::new(4);
        let v0 = VRegIdx::new(0);

        vpr.write_mask_bit(v0, ElemIdx::new(0), false);
        vpr.write_mask_bit(v0, ElemIdx::new(1), true);

        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0xAAAA);
        vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, 0xBBBB);

        let _ = vec_execute(
            VectorOp::VMerge,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(0xCCCC),
            Sew::E32,
            2,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            false, // masked merge
            Vxrm::RoundToNearestUp,
        );

        // Element 0: mask bit=0 → take vs2 = 0xAAAA
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0xAAAA);
        // Element 1: mask bit=1 → take operand1 = 0xCCCC
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 0xCCCC);
    }

    // ── Multiply-accumulate ─────────────────────────────────────────────

    #[test]
    fn test_vmacc() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);

        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 7);
        vpr.write_element(vd, ElemIdx::new(0), Sew::E32, 100);

        // vmacc: vd = vs1 * vs2 + vd = 3 * 7 + 100 = 121
        let _ = run(VectorOp::VMacc, &mut vpr, vd, vs2, VecOperand::Scalar(3), Sew::E32, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 121);
    }

    // ── Extension ───────────────────────────────────────────────────────

    #[test]
    fn test_vsext_vf2() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);

        // Write -1 as E8 (0xFF), sign-extend to E16 should be 0xFFFF
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
        let _ = vec_execute(
            VectorOp::VSextVf2,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(0), // unused for extension ops
            Sew::E16,
            1,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        );
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0xFFFF);
    }

    #[test]
    fn test_vzext_vf2() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);

        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
        let _ = vec_execute(
            VectorOp::VZextVf2,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(0),
            Sew::E16,
            1,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        );
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0x00FF);
    }

    // ── Carry operations ────────────────────────────────────────────────

    #[test]
    fn test_vadc() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let v0 = VRegIdx::new(0);

        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 10);
        vpr.write_mask_bit(v0, ElemIdx::new(0), true); // carry = 1

        let _ = vec_execute(
            VectorOp::VAdc,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(20),
            Sew::E32,
            1,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            false, // use v0 as carry
            Vxrm::RoundToNearestUp,
        );

        // 10 + 20 + 1 (carry) = 31
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 31);
    }

    // ── Narrowing shift ─────────────────────────────────────────────────

    #[test]
    fn test_vnsrl() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);

        // Write 0x1234 at E16, narrow to E8 with shift right by 8
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 0x1234);
        let _ = vec_execute(
            VectorOp::VNSrl,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(8),
            Sew::E8, // destination SEW
            1,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::M1,
            true,
            Vxrm::RoundToNearestUp,
        );
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 0x12);
    }
}

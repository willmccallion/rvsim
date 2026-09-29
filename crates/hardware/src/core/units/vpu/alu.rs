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

use crate::core::units::vpu::regfile::VectorRegFile;
use crate::isa::fp::RoundingMode;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, MaskPolicy, Sew, TailPolicy, VRegIdx, Vlmax, Vlmul, Vxrm};

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
fn mask_active(vpr: &impl VectorRegFile, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Read operand1 value for element `i` at the given SEW, applying the mask.
#[inline]
fn read_op1(vpr: &impl VectorRegFile, operand1: &VecOperand, i: usize, sew: Sew) -> u64 {
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
            let sticky = if d >= 2 { v & ((1u64 << (d - 1)) - 1) } else { 0 };
            let lsb = (v >> d) & 1;
            r & (sticky | lsb)
        }
        Vxrm::RoundDown => 0,
        Vxrm::RoundToOdd => {
            let dropped = v & ((1u64 << d) - 1);
            let result_lsb = (v >> d) & 1;
            if dropped != 0 && result_lsb == 0 { 1 } else { 0 }
        }
    }
}

/// Compute one element for standard (non-widening, non-narrowing) integer ops.
#[inline]
fn compute_standard(op: VectorOp, vs2: u64, op1: u64, sew: Sew, vxrm: Vxrm) -> (u64, bool) {
    let mask = sew.mask();
    let bits = sew.bits();
    let s2 = sign_extend(vs2, sew);
    let s1 = sign_extend(op1, sew);

    match op {
        VectorOp::VAdd => (vs2.wrapping_add(op1) & mask, false),
        VectorOp::VSub => (vs2.wrapping_sub(op1) & mask, false),
        VectorOp::VRsub => (op1.wrapping_sub(vs2) & mask, false),

        VectorOp::VAnd => (vs2 & op1, false),
        VectorOp::VOr => (vs2 | op1, false),
        VectorOp::VXor => (vs2 ^ op1, false),

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

        VectorOp::VMin => ((if s2 < s1 { vs2 } else { op1 }) & mask, false),
        VectorOp::VMinU => (if vs2 < op1 { vs2 } else { op1 }, false),
        VectorOp::VMax => ((if s2 > s1 { vs2 } else { op1 }) & mask, false),
        VectorOp::VMaxU => (if vs2 > op1 { vs2 } else { op1 }, false),

        VectorOp::VMul => (vs2.wrapping_mul(op1) & mask, false),

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
                let neg_one = mask;
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

        VectorOp::VSAddU => {
            let sum = vs2.wrapping_add(op1) & mask;
            if sum < vs2 { (mask, true) } else { (sum, false) }
        }
        VectorOp::VSAdd => {
            let sum = s2 as i128 + s1 as i128;
            if sum > sew.signed_max() as i128 {
                ((sew.signed_max() as u64) & mask, true)
            } else if sum < sew.signed_min() as i128 {
                ((sew.signed_min() as u64) & mask, true)
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
            let diff = s2 as i128 - s1 as i128;
            if diff > sew.signed_max() as i128 {
                ((sew.signed_max() as u64) & mask, true)
            } else if diff < sew.signed_min() as i128 {
                ((sew.signed_min() as u64) & mask, true)
            } else {
                (diff as u64 & mask, false)
            }
        }

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

        VectorOp::VAndN => ((vs2 & !op1) & mask, false),

        VectorOp::VBrev => {
            let v = vs2 & mask;
            let r = bit_reverse(v, bits);
            (r & mask, false)
        }
        VectorOp::VBrev8 => {
            let mut out: u64 = 0;
            let nbytes = bits / 8;
            for i in 0..nbytes {
                let b = ((vs2 >> (i * 8)) & 0xff) as u8;
                out |= u64::from(b.reverse_bits()) << (i * 8);
            }
            (out & mask, false)
        }
        VectorOp::VRev8 => {
            let mut out: u64 = 0;
            let nbytes = bits / 8;
            for i in 0..nbytes {
                let b = (vs2 >> (i * 8)) & 0xff;
                let dst = nbytes - 1 - i;
                out |= b << (dst * 8);
            }
            (out & mask, false)
        }
        VectorOp::VClz => {
            let v = vs2 & mask;
            let lz = if v == 0 { bits as u32 } else { v.leading_zeros() - (64 - bits as u32) };
            (u64::from(lz) & mask, false)
        }
        VectorOp::VCtz => {
            let v = vs2 & mask;
            let tz = if v == 0 { bits as u32 } else { v.trailing_zeros() };
            (u64::from(tz) & mask, false)
        }
        VectorOp::VCpopV => {
            let v = vs2 & mask;
            (u64::from(v.count_ones()) & mask, false)
        }
        VectorOp::VRol => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let v = vs2 & mask;
            let r =
                if shamt == 0 { v } else { ((v << shamt) | (v >> (bits as u32 - shamt))) & mask };
            (r, false)
        }
        VectorOp::VRor => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let v = vs2 & mask;
            let r =
                if shamt == 0 { v } else { ((v >> shamt) | (v << (bits as u32 - shamt))) & mask };
            (r, false)
        }

        // Zvbc: defined only for SEW=64 in the spec; the bit loop generalises.
        VectorOp::VClMul => {
            let a = vs2 & mask;
            let b = op1 & mask;
            (clmul_low(a, b, bits) & mask, false)
        }
        VectorOp::VClMulH => {
            let a = vs2 & mask;
            let b = op1 & mask;
            (clmul_high(a, b, bits) & mask, false)
        }

        _ => unreachable!(),
    }
}

/// Reverse the low `bits` bits of `v`. (`u64::reverse_bits` reverses all 64.)
#[inline]
const fn bit_reverse(v: u64, bits: usize) -> u64 {
    v.reverse_bits() >> (64 - bits)
}

/// Carry-less multiply (GF(2)): low SEW bits of the 2*SEW-bit product.
///
/// For each set bit `i` in `a`, XOR `b << i` into the accumulator.
/// Equivalent to a polynomial multiply over GF(2).
#[inline]
const fn clmul_low(a: u64, b: u64, bits: usize) -> u64 {
    let mut acc: u64 = 0;
    let mut i = 0;
    while i < bits {
        if (a >> i) & 1 != 0 {
            acc ^= b << i;
        }
        i += 1;
    }
    acc
}

/// Carry-less multiply (GF(2)): high SEW bits of the 2*SEW-bit product.
#[inline]
const fn clmul_high(a: u64, b: u64, bits: usize) -> u64 {
    let mut acc: u64 = 0;
    let mut i = 1; // bit 0 of high half corresponds to bit `bits` of full product
    while i < bits {
        if (a >> i) & 1 != 0 {
            // b shifted left by `i`, then we want bits [bits .. 2*bits), so
            // shift right by `bits - i` to land the high half in `acc`.
            acc ^= b >> (bits - i);
        }
        i += 1;
    }
    acc
}

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

        VectorOp::VWsll => {
            let wbits = wsew.bits() as u64;
            let shamt = (op1_val & (wbits - 1)) as u32;
            (u2_narrow << shamt) & wmask
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
            // signed(rs1/vs1) * unsigned(vs2)
            let prod = (sign_extend(op1_val, sew) as i128) * (u2 as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        VectorOp::VWMaccUS => {
            // unsigned(rs1) * signed(vs2)  (.vx form only)
            let prod = (u1 as i128) * (sign_extend(vs2_val, sew) as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        _ => unreachable!(),
    }
}

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
            | VectorOp::VWsll
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
    vpr: &mut impl VectorRegFile,
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
    let ctx = VecExecCtx {
        sew,
        vl,
        vstart,
        vma,
        vta,
        vlmul,
        vm,
        vxrm,
        frm: RoundingMode::Rne,
        zvfh: false,
    };

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

    exec_standard(op, vpr, vd_idx, vs2_idx, operand1, &ctx)
}

/// Standard (non-widening, non-narrowing) element-wise loop.
fn exec_standard(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
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
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let (result, sat) = compute_standard(op, vs2_val, op1_val, ctx.sew, ctx.vxrm);
        vxsat |= sat;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Comparison loop: writes mask bits to vd.
///
/// Mask-producing instructions write one bit per element. The tail comprises
/// bits `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3), so
/// the loop must iterate over all VLEN mask bits, not just VLMAX elements.
fn exec_comparison(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Mask registers hold VLEN bits; the tail extends from vl to VLEN-1.
    let vlen_bits = vpr.vlen().bits();

    for i in 0..vlen_bits {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let result = compute_compare(op, vs2_val, op1_val, ctx.sew);
        vpr.write_mask_bit(vd_idx, ElemIdx::new(i), result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Add/subtract with carry loop.
///
/// For mask-producing variants (`vmadc`, `vmsbc`), the tail comprises bits
/// `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3). For
/// non-mask variants (`vadc`, `vsbc`), the tail is `[vl, VLMAX)` at SEW.
/// We iterate to the larger of the two bounds so both cases are covered.
fn exec_carry(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let vlen_bits = vpr.vlen().bits();
    let mask = ctx.sew.mask();
    let writes_mask = is_mask_producing_carry(op);
    // Mask-producing ops need tail up to VLEN bits; element-producing up to VLMAX.
    let loop_end = if writes_mask { vlen_bits } else { vlmax };

    for i in 0..loop_end {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                if writes_mask {
                    vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
                } else {
                    vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
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

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Multiply-accumulate loop (vmacc, vnmsac, vmadd, vnmsub).
fn exec_macc(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
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
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
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

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Merge/move loop.
///
/// `vmerge.vvm vd, vs2, vs1, v0` — masked merge. When `vm=true` this acts
/// as a simple move of operand1 into vd (all elements from op1).
fn exec_merge(
    vpr: &mut impl VectorRegFile,
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
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
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

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Widening (non-accumulate) loop.
fn exec_widening(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::isa::fp::FpFlags::NONE,
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
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), vs2_sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let result = compute_widening(op, vs2_val, op1_val, ctx.sew, wsew);
        vpr.write_element(vd_idx, ElemIdx::new(i), wsew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Widening multiply-accumulate loop.
fn exec_widening_macc(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::isa::fp::FpFlags::NONE,
        };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_val = vpr.read_element(vd_idx, ElemIdx::new(i), wsew);
        let result = compute_widening_macc(op, vs2_val, op1_val, vd_val, ctx.sew, wsew);
        vpr.write_element(vd_idx, ElemIdx::new(i), wsew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Narrowing loop.
fn exec_narrowing(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::isa::fp::FpFlags::NONE,
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
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
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

    VecExecResult { vxsat, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Extension loop (vzext, vsext).
fn exec_extension(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
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
            fp_flags: crate::isa::fp::FpFlags::NONE,
        };
    };

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let src_val = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);
        let result =
            if is_signed { sign_extend(src_val, src_sew) as u64 & ctx.sew.mask() } else { src_val };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::arch::vpr::Vpr;
    use crate::isa::rvv::Vlen;

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

    /// vwmaccus.vx: vd[i] += unsigned(rs1) * signed(vs2[i]).
    /// vs2[0] = 0x80 (i8 = -128); rs1 = 0xfffffffffffffff8 (low 8 bits 0xf8 = u8 248).
    /// Product = 248 * -128 = -31744 → 0x8400 in u16.
    #[test]
    fn test_vwmaccus_vx_signed_vs2() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0x80);
        let rs1: u64 = (-8_i64) as u64;
        let _ = run(VectorOp::VWMaccUS, &mut vpr, vd, vs2, VecOperand::Scalar(rs1), Sew::E8, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0x8400);
    }

    /// vwmaccsu.vv: vd[i] += signed(vs1[i]) * unsigned(vs2[i]).
    /// vs1[0] = 0x80 (i8 = -128), vs2[0] = 0x80 (u8 = 128). Product = -16384 = 0xc000.
    #[test]
    fn test_vwmaccsu_vv_signed_unsigned() {
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(1);
        let vs1 = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0x80);
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E8, 0x80);
        let _ = run(VectorOp::VWMaccSU, &mut vpr, vd, vs2, VecOperand::Vector(vs1), Sew::E8, 1);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0xc000);
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
    fn test_vwaddu_mf8_widen() {
        // vwaddu.vv at SEW=E8, LMUL=mf8 with vl=2.
        // VLMAX = (128/8)*1/8 = 2 elements at E8. Widens to E16.
        // Source elements vs2[0]=5, vs2[1]=7. Scalar = 3.
        // Expected: vd[0]@E16 = 8, vd[1]@E16 = 10. Tail bytes unchanged (tu).
        let mut vpr = make_vpr();
        let vd = VRegIdx::new(4);
        let vs2 = VRegIdx::new(8);
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 5);
        vpr.write_element(vs2, ElemIdx::new(1), Sew::E8, 7);
        // Sentinel: pre-fill v4 with 0xAA to detect tail clobbering.
        for i in 0..16usize {
            vpr.write_element(vd, ElemIdx::new(i), Sew::E8, 0xAA);
        }

        let _ = vec_execute(
            VectorOp::VWAddU,
            &mut vpr,
            vd,
            vs2,
            VecOperand::Scalar(3),
            Sew::E8,
            2,
            0,
            MaskPolicy::Undisturbed,
            TailPolicy::Undisturbed,
            Vlmul::Mf8,
            true,
            Vxrm::RoundToNearestUp,
        );

        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0x0008);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E16), 0x000a);
        // Tail bytes [4..16] should still be 0xAA (Undisturbed)
        for i in 4..16 {
            assert_eq!(
                vpr.read_element(vd, ElemIdx::new(i), Sew::E8),
                0xAA,
                "tail byte {i} clobbered"
            );
        }
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
        // Tail element with agnostic: written with all-1s per RVV 1.0
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), Sew::E32.ones());
    }

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

//! Single-width integer and fixed-point arithmetic, multiply-accumulate,
//! merges, extensions and carry-less multiplies.

use crate::exec::compute::vector::alu::frac_sew;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1, sign_extend,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax, Vxrm};

/// Compute the fixed-point rounding increment for averaging / scaling ops.
///
/// Per the RVV spec, the rounding bit `r` depends on `vxrm` and the value
/// being shifted. `v` is the full pre-shift value, `d` is the shift amount.
#[inline]
pub(super) const fn rounding_incr(v: u64, d: u32, vxrm: Vxrm) -> u64 {
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
pub(super) fn compute_standard(
    op: VectorOp,
    vs2: u64,
    op1: u64,
    sew: Sew,
    vxrm: Vxrm,
) -> (u64, bool) {
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

        VectorOp::VDivU => (vs2.checked_div(op1).map_or(mask, |q| q & mask), false),
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
pub(super) const fn bit_reverse(v: u64, bits: usize) -> u64 {
    v.reverse_bits() >> (64 - bits)
}

/// Carry-less multiply (GF(2)): low SEW bits of the 2*SEW-bit product.
///
/// For each set bit `i` in `a`, XOR `b << i` into the accumulator.
/// Equivalent to a polynomial multiply over GF(2).
#[inline]
pub(super) const fn clmul_low(a: u64, b: u64, bits: usize) -> u64 {
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
pub(super) const fn clmul_high(a: u64, b: u64, bits: usize) -> u64 {
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

/// Standard (non-widening, non-narrowing) element-wise loop.
pub(super) fn exec_standard(
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

/// Multiply-accumulate loop (vmacc, vnmsac, vmadd, vnmsub).
pub(super) fn exec_macc(
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
pub(super) fn exec_merge(
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

/// Extension loop (vzext, vsext).
pub(super) fn exec_extension(
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

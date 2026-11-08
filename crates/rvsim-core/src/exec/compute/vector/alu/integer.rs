//! Single-width integer and fixed-point arithmetic, multiply-accumulate,
//! merges, extensions and carry-less multiplies.

use crate::exec::compute::clmul::{clmul_high, clmul_low};
use crate::exec::compute::vector::alu::frac_sew;
use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::{ExtendOp, IntOp, MaccOp, Signedness};
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
pub(super) fn compute_standard(op: IntOp, vs2: u64, op1: u64, sew: Sew, vxrm: Vxrm) -> (u64, bool) {
    let mask = sew.mask();
    let bits = sew.bits();
    let s2 = sew.sign_extend(vs2);
    let s1 = sew.sign_extend(op1);

    match op {
        IntOp::Add => (vs2.wrapping_add(op1) & mask, false),
        IntOp::Sub => (vs2.wrapping_sub(op1) & mask, false),
        IntOp::Rsub => (op1.wrapping_sub(vs2) & mask, false),

        IntOp::And => (vs2 & op1, false),
        IntOp::Or => (vs2 | op1, false),
        IntOp::Xor => (vs2 ^ op1, false),

        IntOp::Sll => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            ((vs2 << shamt) & mask, false)
        }
        IntOp::Srl => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            ((vs2 >> shamt) & mask, false)
        }
        IntOp::Sra => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let result = (s2 >> shamt).cast_unsigned();
            (result & mask, false)
        }

        IntOp::Min => ((if s2 < s1 { vs2 } else { op1 }) & mask, false),
        IntOp::MinU => (if vs2 < op1 { vs2 } else { op1 }, false),
        IntOp::Max => ((if s2 > s1 { vs2 } else { op1 }) & mask, false),
        IntOp::MaxU => (if vs2 > op1 { vs2 } else { op1 }, false),

        IntOp::Mul => (vs2.wrapping_mul(op1) & mask, false),

        IntOp::Mulh => {
            let prod = i128::from(s2).wrapping_mul(i128::from(s1));
            let hi = (prod >> bits) as u64;
            (hi & mask, false)
        }
        IntOp::Mulhu => {
            let prod = u128::from(vs2).wrapping_mul(u128::from(op1));
            let hi = (prod >> bits) as u64;
            (hi & mask, false)
        }
        IntOp::Mulhsu => {
            let prod = i128::from(s2).wrapping_mul(i128::from(op1));
            let hi = (prod >> bits) as u64;
            (hi & mask, false)
        }

        IntOp::DivU => (vs2.checked_div(op1).map_or(mask, |q| q & mask), false),
        IntOp::Div => {
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
                    let result = s2.wrapping_div(s1).cast_unsigned();
                    (result & mask, false)
                }
            }
        }
        IntOp::RemU => {
            if op1 == 0 {
                (vs2, false)
            } else {
                ((vs2 % op1) & mask, false)
            }
        }
        IntOp::Rem => {
            if op1 == 0 {
                (vs2, false)
            } else {
                let min_int = 1u64 << (bits - 1);
                let neg_one = mask;
                if vs2 == min_int && op1 == neg_one {
                    // signed overflow: MIN_INT % -1 = 0
                    (0, false)
                } else {
                    let result = s2.wrapping_rem(s1).cast_unsigned();
                    (result & mask, false)
                }
            }
        }

        IntOp::SAddU => {
            let sum = vs2.wrapping_add(op1) & mask;
            if sum < vs2 { (mask, true) } else { (sum, false) }
        }
        IntOp::SAdd => {
            let sum = i128::from(s2) + i128::from(s1);
            if sum > i128::from(sew.signed_max()) {
                (sew.signed_max().cast_unsigned() & mask, true)
            } else if sum < i128::from(sew.signed_min()) {
                (sew.signed_min().cast_unsigned() & mask, true)
            } else {
                (sum as u64 & mask, false)
            }
        }
        IntOp::SSubU => {
            if vs2 < op1 {
                (0, true)
            } else {
                (vs2.wrapping_sub(op1) & mask, false)
            }
        }
        IntOp::SSub => {
            let diff = i128::from(s2) - i128::from(s1);
            if diff > i128::from(sew.signed_max()) {
                (sew.signed_max().cast_unsigned() & mask, true)
            } else if diff < i128::from(sew.signed_min()) {
                (sew.signed_min().cast_unsigned() & mask, true)
            } else {
                (diff as u64 & mask, false)
            }
        }

        IntOp::AAddU => {
            let sum = u128::from(vs2) + u128::from(op1);
            let r = rounding_incr(sum as u64, 1, vxrm);
            let result = ((sum >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }
        IntOp::AAdd => {
            let sum = i128::from(s2) + i128::from(s1);
            let r = rounding_incr(sum as u64, 1, vxrm);
            let result = ((sum >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }
        IntOp::ASubU => {
            let diff = i128::from(vs2) - i128::from(op1);
            let r = rounding_incr(diff as u64, 1, vxrm);
            let result = ((diff >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }
        IntOp::ASub => {
            let diff = i128::from(s2) - i128::from(s1);
            let r = rounding_incr(diff as u64, 1, vxrm);
            let result = ((diff >> 1) as u64).wrapping_add(r);
            (result & mask, false)
        }

        IntOp::Smul => {
            let prod = i128::from(s2) * i128::from(s1);
            let shift = bits - 1;
            let r = rounding_incr(prod as u64, shift as u32, vxrm);
            let result_wide = (prod >> shift) + i128::from(r);
            let max_pos = (1i64 << (bits - 1)) - 1;
            let min_neg = -(1i64 << (bits - 1));
            let sat;
            let clamped = if result_wide > i128::from(max_pos) {
                sat = true;
                max_pos.cast_unsigned()
            } else if result_wide < i128::from(min_neg) {
                sat = true;
                min_neg.cast_unsigned()
            } else {
                sat = false;
                result_wide as u64
            };
            (clamped & mask, sat)
        }

        IntOp::SSrl => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let r = rounding_incr(vs2, shamt, vxrm);
            let result = (vs2 >> shamt).wrapping_add(r);
            (result & mask, false)
        }
        IntOp::SSra => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let r = rounding_incr(vs2, shamt, vxrm);
            let result = (s2 >> shamt).cast_unsigned().wrapping_add(r);
            (result & mask, false)
        }

        IntOp::AndN => ((vs2 & !op1) & mask, false),

        IntOp::Brev => {
            let v = vs2 & mask;
            let r = bit_reverse(v, bits);
            (r & mask, false)
        }
        // Reversing every bit, then the bytes, reverses each byte in place.
        IntOp::Brev8 => ((vs2 & mask).reverse_bits().swap_bytes() & mask, false),
        IntOp::Rev8 => ((vs2 & mask).swap_bytes() >> (64 - bits), false),
        IntOp::Clz => {
            let v = vs2 & mask;
            let lz = if v == 0 { bits as u32 } else { v.leading_zeros() - (64 - bits as u32) };
            (u64::from(lz) & mask, false)
        }
        IntOp::Ctz => {
            let v = vs2 & mask;
            let tz = if v == 0 { bits as u32 } else { v.trailing_zeros() };
            (u64::from(tz) & mask, false)
        }
        IntOp::CpopV => {
            let v = vs2 & mask;
            (u64::from(v.count_ones()) & mask, false)
        }
        IntOp::Rol => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let v = vs2 & mask;
            let r =
                if shamt == 0 { v } else { ((v << shamt) | (v >> (bits as u32 - shamt))) & mask };
            (r, false)
        }
        IntOp::Ror => {
            let shamt = (op1 & (bits as u64 - 1)) as u32;
            let v = vs2 & mask;
            let r =
                if shamt == 0 { v } else { ((v >> shamt) | (v << (bits as u32 - shamt))) & mask };
            (r, false)
        }

        // Zvbc: defined only for SEW=64 in the spec; the bit loop generalises.
        IntOp::ClMul => {
            let a = vs2 & mask;
            let b = op1 & mask;
            (clmul_low(a, b, bits) & mask, false)
        }
        IntOp::ClMulH => {
            let a = vs2 & mask;
            let b = op1 & mask;
            (clmul_high(a, b, bits) & mask, false)
        }
    }
}

/// Reverse the low `bits` bits of `v`. (`u64::reverse_bits` reverses all 64.)
#[inline]
pub(super) const fn bit_reverse(v: u64, bits: usize) -> u64 {
    v.reverse_bits() >> (64 - bits)
}

/// Standard (non-widening, non-narrowing) element-wise loop.
pub(super) fn exec_standard(
    op: IntOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut vxsat = false;

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
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
    op: MaccOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mask = ctx.sew.mask();

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_val = vpr.read_element(vd_idx, ElemIdx::new(i), ctx.sew);

        let result = match op {
            // vd = vs1 * vs2 + vd
            MaccOp::Macc => op1_val.wrapping_mul(vs2_val).wrapping_add(vd_val) & mask,
            // vd = -(vs1 * vs2) + vd
            MaccOp::NMSac => vd_val.wrapping_sub(op1_val.wrapping_mul(vs2_val)) & mask,
            // vd = vs1 * vd + vs2
            MaccOp::Madd => op1_val.wrapping_mul(vd_val).wrapping_add(vs2_val) & mask,
            // vd = -(vs1 * vd) + vs2
            MaccOp::NMSub => vs2_val.wrapping_sub(op1_val.wrapping_mul(vd_val)) & mask,
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult::default()
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
        if ctx.skip_outside_body(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
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

    VecExecResult::default()
}

/// Extension loop (vzext, vsext).
pub(super) fn exec_extension(
    op: ExtendOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let (factor, is_signed) =
        (usize::from(op.factor.divisor()), op.signedness == Signedness::Signed);

    let Some(src_sew) = frac_sew(ctx.sew, factor) else {
        return VecExecResult {
            vxsat: false,
            scalar_result: None,
            fp_flags: crate::isa::fp::FpFlags::NONE,
        };
    };

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
            continue;
        }

        let src_val = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);
        let result = if is_signed {
            src_sew.sign_extend(src_val).cast_unsigned() & ctx.sew.mask()
        } else {
            src_val
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult::default()
}

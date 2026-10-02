//! Widening and narrowing integer operations.

use crate::exec::compute::vector::alu::integer::rounding_incr;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1, sign_extend, widen_sew,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::{NarrowOp, WidenMaccOp, WidenOp};
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax, Vxrm};

/// Compute one widening element. Reads sources at `sew`, writes at `wsew`.
/// For `.w` variants, vs2 is already at `wsew`.
#[inline]
pub(super) const fn compute_widening(
    op: WidenOp,
    vs2_val: u64,
    op1_val: u64,
    sew: Sew,
    wsew: Sew,
) -> u64 {
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
        WidenOp::AddU => u2_narrow.wrapping_add(u1) & wmask,
        WidenOp::Add => s2_narrow.wrapping_add(s1) & wmask,
        WidenOp::SubU => u2_narrow.wrapping_sub(u1) & wmask,
        WidenOp::Sub => s2_narrow.wrapping_sub(s1) & wmask,

        WidenOp::AddUW => u2_wide.wrapping_add(u1) & wmask,
        WidenOp::AddW => s2_wide.wrapping_add(s1) & wmask,
        WidenOp::SubUW => u2_wide.wrapping_sub(u1) & wmask,
        WidenOp::SubW => s2_wide.wrapping_sub(s1) & wmask,

        WidenOp::MulU => {
            let prod = (u2_narrow as u128) * (u1 as u128);
            prod as u64 & wmask
        }
        WidenOp::Mul => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (sign_extend(op1_val, sew) as i128);
            prod as u64 & wmask
        }
        WidenOp::MulSU => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (u1 as i128);
            prod as u64 & wmask
        }

        WidenOp::Sll => {
            let wbits = wsew.bits() as u64;
            let shamt = (op1_val & (wbits - 1)) as u32;
            (u2_narrow << shamt) & wmask
        }
    }
}

/// Compute one widening multiply-accumulate element.
#[inline]
pub(super) const fn compute_widening_macc(
    op: WidenMaccOp,
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
        WidenMaccOp::MaccU => {
            let prod = (u2 as u128) * (u1 as u128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        WidenMaccOp::Macc => {
            let prod = (sign_extend(vs2_val, sew) as i128) * (sign_extend(op1_val, sew) as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        WidenMaccOp::MaccSU => {
            // signed(rs1/vs1) * unsigned(vs2)
            let prod = (sign_extend(op1_val, sew) as i128) * (u2 as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
        WidenMaccOp::MaccUS => {
            // unsigned(rs1) * signed(vs2)  (.vx form only)
            let prod = (u1 as i128) * (sign_extend(vs2_val, sew) as i128);
            (prod as u64).wrapping_add(acc) & wmask
        }
    }
}

/// Compute one narrowing element. Reads vs2 at `wsew` (2*SEW), shift amount
/// from op1 at `sew`, writes result at `sew`.
#[inline]
pub(super) const fn compute_narrowing(
    op: NarrowOp,
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
        NarrowOp::Srl => {
            let result = vs2_val >> shamt;
            (result & mask, false)
        }
        NarrowOp::Sra => {
            let s = sign_extend(vs2_val, wsew);
            let result = (s >> shamt) as u64;
            (result & mask, false)
        }
        NarrowOp::ClipU => {
            let r = rounding_incr(vs2_val, shamt, vxrm);
            let shifted = (vs2_val >> shamt).wrapping_add(r);
            if shifted > mask { (mask, true) } else { (shifted & mask, false) }
        }
        NarrowOp::Clip => {
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
    }
}

/// Widening (non-accumulate) loop.
pub(super) fn exec_widening(
    op: WidenOp,
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
    let vs2_sew = if op.reads_wide_vs2() { wsew } else { ctx.sew };

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
pub(super) fn exec_widening_macc(
    op: WidenMaccOp,
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
pub(super) fn exec_narrowing(
    op: NarrowOp,
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

//! Vector reduction operations.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) reduction instructions.
//! Reductions fold a vector register group (vs2) and an initial scalar
//! accumulator (element 0 of vs1) into a single result in element 0 of vd.
//!
//! Operations handled:
//! - **Integer:** `vredsum`, `vredand`, `vredor`, `vredxor`, `vredminu`,
//!   `vredmin`, `vredmaxu`, `vredmax`
//! - **Widening integer:** `vwredsumu`, `vwredsum`
//! - **FP:** `vfredosum`, `vfredusum`, `vfredmax`, `vfredmin`
//! - **FP widening:** `vfwredosum`, `vfwredusum`
//!
//! All reductions read the initial accumulator from `vs1[0]` and write the
//! result to `vd[0]`; remaining elements of `vd` follow the tail policy.

// IEEE 754 FEQ requires exact bit-pattern comparison — float_cmp is intentional here.
#![allow(clippy::float_cmp)]

use crate::core::pipeline::signals::VectorOp;
use crate::core::units::fpu::exception_flags::FpFlags;
use crate::core::units::fpu::nan_handling::{
    box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
};
use crate::core::units::fpu::{clear_host_fp_flags, read_host_fp_flags};
use crate::core::units::vpu::alu::{VecExecCtx, VecExecResult, VecOperand};
use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::vpu::types::{ElemIdx, Sew, VRegIdx, Vlmax, Vlmul};

// ============================================================================
// Public API
// ============================================================================

/// Returns `true` if `op` is a reduction handled by this module.
pub const fn is_reduction(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VRedSum
            | VectorOp::VRedAnd
            | VectorOp::VRedOr
            | VectorOp::VRedXor
            | VectorOp::VRedMinU
            | VectorOp::VRedMin
            | VectorOp::VRedMaxU
            | VectorOp::VRedMax
            | VectorOp::VWRedSumU
            | VectorOp::VWRedSum
            | VectorOp::VFRedOSum
            | VectorOp::VFRedUSum
            | VectorOp::VFRedMax
            | VectorOp::VFRedMin
            | VectorOp::VFWRedOSum
            | VectorOp::VFWRedUSum
    )
}

/// Execute a reduction operation.
///
/// The initial accumulator is extracted from `operand1` (which carries vs1[0]).
/// Results are written to `vd[0]` in the VPR; remaining elements follow tail policy.
///
/// # Integer reductions
///
/// `VRedSum`, `VRedAnd`, `VRedOr`, `VRedXor`, `VRedMinU`, `VRedMin`,
/// `VRedMaxU`, `VRedMax` operate at SEW width.
///
/// # Widening integer reductions
///
/// `VWRedSumU` and `VWRedSum` read source elements at SEW, extend to 2*SEW,
/// and accumulate at 2*SEW width.
///
/// # FP reductions
///
/// `VFRedOSum`, `VFRedUSum`, `VFRedMax`, `VFRedMin` operate at SEW (32 or 64).
/// Ordered sums process elements sequentially; unordered sums use the same
/// sequential ordering for deterministic results.
///
/// # FP widening reductions
///
/// `VFWRedOSum`, `VFWRedUSum` read source elements at SEW (E32), widen to
/// f64, and accumulate at 2*SEW (E64).
#[allow(clippy::too_many_arguments)]
pub fn vec_reduce(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        // Integer reductions at SEW width.
        VectorOp::VRedSum
        | VectorOp::VRedAnd
        | VectorOp::VRedOr
        | VectorOp::VRedXor
        | VectorOp::VRedMinU
        | VectorOp::VRedMin
        | VectorOp::VRedMaxU
        | VectorOp::VRedMax => exec_int_reduction(op, vpr, vd, vs2, operand1, ctx),

        // Widening integer reductions (accumulator at 2*SEW).
        VectorOp::VWRedSumU | VectorOp::VWRedSum => {
            exec_widen_int_reduction(op, vpr, vd, vs2, operand1, ctx)
        }

        // FP reductions at SEW width.
        VectorOp::VFRedOSum | VectorOp::VFRedUSum | VectorOp::VFRedMax | VectorOp::VFRedMin => {
            exec_fp_reduction(op, vpr, vd, vs2, operand1, ctx)
        }

        // FP widening reductions (accumulator at 2*SEW).
        VectorOp::VFWRedOSum | VectorOp::VFWRedUSum => {
            exec_fp_widen_reduction(op, vpr, vd, vs2, operand1, ctx)
        }

        _ => unreachable!("vec_reduce called with non-reduction op: {:?}", op),
    }
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

/// Read v0 mask bit for element `i`.
#[inline]
fn mask_active(vpr: &impl VectorRegFile, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Write all-1s at the given SEW width to a destination element.
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

/// Read the initial accumulator value from `operand1` at the given element width.
///
/// For `VecOperand::Vector(vs1)` this reads element 0 from the vector register.
/// For scalar/immediate operands the value is masked to the element width.
#[inline]
fn read_initial_accum(vpr: &impl VectorRegFile, operand1: &VecOperand, sew: Sew) -> u64 {
    match operand1 {
        VecOperand::Vector(vs1) => vpr.read_element(*vs1, ElemIdx::new(0), sew),
        VecOperand::Scalar(s) => *s & sew.mask(),
        VecOperand::Immediate(imm) => (*imm as u64) & sew.mask(),
    }
}

// ============================================================================
// Integer reductions
// ============================================================================

/// Execute an integer reduction at SEW width.
///
/// Folds all active elements of `vs2` into a single accumulator (initialized
/// from `vs1[0]`) using the operation specified by `op`. The result is written
/// to `vd[0]`; elements `1..vlmax` of `vd` follow the tail policy.
fn exec_int_reduction(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let sew = ctx.sew;
    let mask = sew.mask();

    // Read initial accumulator from vs1[0].
    let mut acc = read_initial_accum(vpr, operand1, sew);

    // Fold active elements from vs2.
    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem = vpr.read_element(vs2, ElemIdx::new(i), sew);
        acc = int_reduce_step(op, acc, elem, sew, mask);
    }

    // Write result to vd[0].
    vpr.write_element(vd, ElemIdx::new(0), sew, acc & mask);

    // Tail policy: elements 1..vlmax of vd follow the tail-agnostic rule.
    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.write_element(vd, ElemIdx::new(i), sew, sew.ones());
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}

/// Perform one step of an integer reduction.
///
/// Combines the current accumulator with a new element according to the
/// reduction operation.
#[inline]
fn int_reduce_step(op: VectorOp, acc: u64, elem: u64, sew: Sew, mask: u64) -> u64 {
    match op {
        VectorOp::VRedSum => acc.wrapping_add(elem) & mask,
        VectorOp::VRedAnd => acc & elem,
        VectorOp::VRedOr => acc | elem,
        VectorOp::VRedXor => acc ^ elem,
        VectorOp::VRedMinU => {
            if elem < acc {
                elem
            } else {
                acc
            }
        }
        VectorOp::VRedMin => {
            let sa = sign_extend(acc, sew);
            let se = sign_extend(elem, sew);
            if se < sa { elem } else { acc }
        }
        VectorOp::VRedMaxU => {
            if elem > acc {
                elem
            } else {
                acc
            }
        }
        VectorOp::VRedMax => {
            let sa = sign_extend(acc, sew);
            let se = sign_extend(elem, sew);
            if se > sa { elem } else { acc }
        }
        _ => unreachable!(),
    }
}

// ============================================================================
// Widening integer reductions
// ============================================================================

/// Execute a widening integer reduction.
///
/// Source elements are read at SEW and extended to 2*SEW before accumulation.
/// The accumulator (from `vs1[0]`) and the result are at 2*SEW width.
fn exec_widen_int_reduction(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let src_sew = ctx.sew;
    let Some(dst_sew) = widen_sew(src_sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    let dst_mask = dst_sew.mask();

    // Initial accumulator is at 2*SEW from vs1[0].
    let mut acc = read_initial_accum(vpr, operand1, dst_sew);

    // Fold active source elements.
    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem = vpr.read_element(vs2, ElemIdx::new(i), src_sew);

        // Extend to 2*SEW width.
        let wide = match op {
            VectorOp::VWRedSumU => {
                // Zero-extend: element already fits in u64 with high bits clear.
                elem
            }
            VectorOp::VWRedSum => {
                // Sign-extend from src_sew to full 64 bits, then mask to dst_sew.
                (sign_extend(elem, src_sew) as u64) & dst_mask
            }
            _ => unreachable!(),
        };

        acc = acc.wrapping_add(wide) & dst_mask;
    }

    // Write result to vd[0] at 2*SEW.
    vpr.write_element(vd, ElemIdx::new(0), dst_sew, acc);

    // Tail policy: elements 1..vlmax of vd follow the tail-agnostic rule.
    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), dst_sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.write_element(vd, ElemIdx::new(i), dst_sew, dst_sew.ones());
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}

// ============================================================================
// FP reductions
// ============================================================================

/// Execute a floating-point reduction at SEW width (32 or 64).
///
/// For ordered sums (`VFRedOSum`) elements are processed sequentially from
/// element 0 to `vl-1`. Unordered sums (`VFRedUSum`) use the same sequential
/// ordering for deterministic results.
///
/// FP min/max reductions (`VFRedMin`, `VFRedMax`) use IEEE 754-2008 minNum/maxNum
/// semantics, matching the scalar `fmin`/`fmax` helpers.
fn exec_fp_reduction(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let sew = ctx.sew;

    let flags = match sew {
        Sew::E32 => {
            let (result_bits, flags) = fp_reduce_f32(op, vpr, vs2, operand1, ctx);
            vpr.write_element(vd, ElemIdx::new(0), sew, result_bits);
            flags
        }
        Sew::E64 => {
            let (result_bits, flags) = fp_reduce_f64(op, vpr, vs2, operand1, ctx);
            vpr.write_element(vd, ElemIdx::new(0), sew, result_bits);
            flags
        }
        _ => unreachable!("FP reduction with SEW={:?} is not supported", sew),
    };

    // Tail policy: elements 1..vlmax of vd follow the tail-agnostic rule.
    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.write_element(vd, ElemIdx::new(i), sew, sew.ones());
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Single-precision (f32) reduction loop.
///
/// Returns `(result_bits, fp_flags)` where `result_bits` is the NaN-boxed
/// canonical f32 result suitable for writing at SEW=32.
fn fp_reduce_f32(
    op: VectorOp,
    vpr: &impl VectorRegFile,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> (u64, FpFlags) {
    let sew = ctx.sew;
    let init_bits = read_initial_accum(vpr, operand1, sew);
    let mut acc = f32::from_bits(init_bits as u32);
    let mut flags = FpFlags::NONE;

    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem_bits = vpr.read_element(vs2, ElemIdx::new(i), sew);
        let elem = f32::from_bits(elem_bits as u32);

        match op {
            VectorOp::VFRedOSum | VectorOp::VFRedUSum => {
                clear_host_fp_flags();
                acc = std::hint::black_box(std::hint::black_box(acc) + std::hint::black_box(elem));
                flags = flags | read_host_fp_flags();
            }
            VectorOp::VFRedMin => {
                acc = fmin_f32(acc, elem);
            }
            VectorOp::VFRedMax => {
                acc = fmax_f32(acc, elem);
            }
            _ => unreachable!(),
        }
    }

    (box_f32_canon(acc), flags)
}

/// Double-precision (f64) reduction loop.
///
/// Returns `(result_bits, fp_flags)` where `result_bits` is the canonicalized
/// f64 bit pattern suitable for writing at SEW=64.
fn fp_reduce_f64(
    op: VectorOp,
    vpr: &impl VectorRegFile,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> (u64, FpFlags) {
    let sew = ctx.sew;
    let init_bits = read_initial_accum(vpr, operand1, sew);
    let mut acc = f64::from_bits(init_bits);
    let mut flags = FpFlags::NONE;

    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem_bits = vpr.read_element(vs2, ElemIdx::new(i), sew);
        let elem = f64::from_bits(elem_bits);

        match op {
            VectorOp::VFRedOSum | VectorOp::VFRedUSum => {
                clear_host_fp_flags();
                acc = std::hint::black_box(std::hint::black_box(acc) + std::hint::black_box(elem));
                flags = flags | read_host_fp_flags();
            }
            VectorOp::VFRedMin => {
                acc = fmin_f64(acc, elem);
            }
            VectorOp::VFRedMax => {
                acc = fmax_f64(acc, elem);
            }
            _ => unreachable!(),
        }
    }

    (canonicalize_f64_bits(acc), flags)
}

// ============================================================================
// FP widening reductions
// ============================================================================

/// Execute a widening FP reduction.
///
/// Source elements are at SEW (must be E32), accumulator is at 2*SEW (E64).
/// Elements are converted from f32 to f64 before accumulation.
fn exec_fp_widen_reduction(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let src_sew = ctx.sew;
    let Some(dst_sew) = widen_sew(src_sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    // Initial accumulator at 2*SEW (f64).
    let init_bits = read_initial_accum(vpr, operand1, dst_sew);
    let mut acc = f64::from_bits(init_bits);
    let mut flags = FpFlags::NONE;

    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }

        let elem_bits = vpr.read_element(vs2, ElemIdx::new(i), src_sew);

        // Widen the source element to f64.
        let wide = match src_sew {
            Sew::E32 => f32::from_bits(elem_bits as u32) as f64,
            _ => unreachable!("widening FP reduction source must be E32, got {:?}", src_sew),
        };

        match op {
            VectorOp::VFWRedOSum | VectorOp::VFWRedUSum => {
                clear_host_fp_flags();
                acc = std::hint::black_box(std::hint::black_box(acc) + std::hint::black_box(wide));
                flags = flags | read_host_fp_flags();
            }
            _ => unreachable!(),
        }
    }

    // Write result at 2*SEW.
    vpr.write_element(vd, ElemIdx::new(0), dst_sew, canonicalize_f64_bits(acc));

    // Tail policy: elements 1..vlmax of vd follow the tail-agnostic rule.
    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), dst_sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.write_element(vd, ElemIdx::new(i), dst_sew, dst_sew.ones());
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::arch::vpr::Vpr;
    use crate::core::units::vpu::types::{MaskPolicy, TailPolicy, Vlen, Vlmul, Vxrm};

    /// Create a standard execution context with the given SEW and vl.
    fn make_ctx(sew: Sew, vl: usize) -> VecExecCtx {
        VecExecCtx {
            sew,
            vl,
            vstart: 0,
            vma: MaskPolicy::Undisturbed,
            vta: TailPolicy::Undisturbed,
            vlmul: Vlmul::M1,
            vm: true,
            vxrm: Vxrm::RoundToNearestUp,
        }
    }

    /// Create a 128-bit VLEN vector register file.
    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    #[test]
    fn test_vredsum() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        // vs2 = [10, 20, 30, 40], vs1[0] = 100 (accumulator)
        for i in 0..4 {
            vpr.write_element(vs2, ElemIdx::new(i), Sew::E32, (i as u64 + 1) * 10);
        }
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, 100);

        let operand1 = VecOperand::Vector(vs1);
        let _result = vec_reduce(VectorOp::VRedSum, &mut vpr, vd, vs2, &operand1, &ctx);
        // 100 + 10 + 20 + 30 + 40 = 200
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 200);
    }

    #[test]
    fn test_vredand() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        for i in 0..4 {
            vpr.write_element(vs2, ElemIdx::new(i), Sew::E32, 0xFF);
        }
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, 0xFFFF_FFFF);

        let operand1 = VecOperand::Vector(vs1);
        let _result = vec_reduce(VectorOp::VRedAnd, &mut vpr, vd, vs2, &operand1, &ctx);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0xFF);
    }

    #[test]
    fn test_vredmin_signed() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        // vs2 = [5, -3, 10, 1] as signed i32
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 5);
        vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, (-3i32 as u32) as u64);
        vpr.write_element(vs2, ElemIdx::new(2), Sew::E32, 10);
        vpr.write_element(vs2, ElemIdx::new(3), Sew::E32, 1);
        // accumulator = 100
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, 100);

        let operand1 = VecOperand::Vector(vs1);
        let _result = vec_reduce(VectorOp::VRedMin, &mut vpr, vd, vs2, &operand1, &ctx);
        // min(100, 5, -3, 10, 1) = -3
        let val = vpr.read_element(vd, ElemIdx::new(0), Sew::E32);
        assert_eq!(val as u32, (-3i32) as u32);
    }

    #[test]
    fn test_vfredosum_f32() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        // vs2 = [1.0, 2.0, 3.0, 4.0], accum = 10.0
        for i in 0..4 {
            vpr.write_element(vs2, ElemIdx::new(i), Sew::E32, ((i as f32 + 1.0).to_bits()) as u64);
        }
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, 10.0f32.to_bits() as u64);

        let operand1 = VecOperand::Vector(vs1);
        let _result = vec_reduce(VectorOp::VFRedOSum, &mut vpr, vd, vs2, &operand1, &ctx);
        let val = f32::from_bits(vpr.read_element(vd, ElemIdx::new(0), Sew::E32) as u32);
        assert_eq!(val, 20.0); // 10 + 1 + 2 + 3 + 4
    }
}

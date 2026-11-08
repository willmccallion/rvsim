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

use crate::exec::compute::fpu::float::SoftF16;
use crate::exec::compute::fpu::host::{restore_host_round_mode, set_host_round_mode};
use crate::exec::compute::fpu::invalid_if_signaling;
use crate::exec::compute::vector::context::{
    FpSew, FpWiden, VecExecCtx, VecExecResult, mask_active, widen_sew,
};
use crate::exec::compute::vector::fpu::VecFloat;
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::{FpReduceOp, FpWidenReduceOp, IntReduceOp, ReduceOp, WidenIntReduceOp};
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax, Vlmul};

/// Execute a reduction of `vs2` seeded from `vs1[0]`.
///
/// Results are written to `vd[0]` in the VPR; remaining elements follow the
/// tail policy.
///
/// # Integer reductions
///
/// Operate at SEW width.
///
/// # Widening integer reductions
///
/// Read source elements at SEW, extend to 2*SEW, and accumulate at 2*SEW.
///
/// # FP reductions
///
/// Operate at SEW (16 with Zvfh, 32 or 64). Ordered sums process elements
/// sequentially; unordered sums use the same sequential ordering for
/// deterministic results.
///
/// # FP widening reductions
///
/// Read source elements at SEW, widen, and accumulate at 2*SEW.
pub fn vec_reduce(
    op: ReduceOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        ReduceOp::Int(op) => exec_int_reduction(op, vpr, vd, vs2, vs1, ctx),
        ReduceOp::WidenInt(op) => exec_widen_int_reduction(op, vpr, vd, vs2, vs1, ctx),
        ReduceOp::Fp(op) => exec_fp_reduction(op, vpr, vd, vs2, vs1, ctx),
        ReduceOp::FpWiden(op) => exec_fp_widen_reduction(op, vpr, vd, vs2, vs1, ctx),
    }
}

/// The initial accumulator: element 0 of `vs1` at `sew`.
#[inline]
fn read_initial_accum(vpr: &impl VectorRegFile, vs1: VRegIdx, sew: Sew) -> u64 {
    vpr.read_element(vs1, ElemIdx::new(0), sew)
}

/// Execute an integer reduction at SEW width.
///
/// Folds all active elements of `vs2` into a single accumulator (initialized
/// from `vs1[0]`) using the operation specified by `op`. The result is written
/// to `vd[0]`; elements `1..vlmax` of `vd` follow the tail policy.
fn exec_int_reduction(
    op: IntReduceOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let sew = ctx.sew;
    let mask = sew.mask();

    let mut acc = read_initial_accum(vpr, vs1, sew);

    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem = vpr.read_element(vs2, ElemIdx::new(i), sew);
        acc = int_reduce_step(op, acc, elem, sew, mask);
    }

    vpr.write_element(vd, ElemIdx::new(0), sew, acc & mask);

    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.fill_agnostic_element(vd, ElemIdx::new(i), sew);
        }
    }

    VecExecResult::default()
}

/// Perform one step of an integer reduction.
///
/// Combines the current accumulator with a new element according to the
/// reduction operation.
#[inline]
const fn int_reduce_step(op: IntReduceOp, acc: u64, elem: u64, sew: Sew, mask: u64) -> u64 {
    match op {
        IntReduceOp::Sum => acc.wrapping_add(elem) & mask,
        IntReduceOp::And => acc & elem,
        IntReduceOp::Or => acc | elem,
        IntReduceOp::Xor => acc ^ elem,
        IntReduceOp::MinU => {
            if elem < acc {
                elem
            } else {
                acc
            }
        }
        IntReduceOp::Min => {
            let sa = sew.sign_extend(acc);
            let se = sew.sign_extend(elem);
            if se < sa { elem } else { acc }
        }
        IntReduceOp::MaxU => {
            if elem > acc {
                elem
            } else {
                acc
            }
        }
        IntReduceOp::Max => {
            let sa = sew.sign_extend(acc);
            let se = sew.sign_extend(elem);
            if se > sa { elem } else { acc }
        }
    }
}

/// Execute a widening integer reduction.
///
/// Source elements are read at SEW and extended to 2*SEW before accumulation.
/// The accumulator (from `vs1[0]`) and the result are at 2*SEW width.
fn exec_widen_int_reduction(
    op: WidenIntReduceOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let src_sew = ctx.sew;
    let Some(dst_sew) = widen_sew(src_sew) else {
        return VecExecResult::default();
    };
    let dst_mask = dst_sew.mask();

    let mut acc = read_initial_accum(vpr, vs1, dst_sew);

    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem = vpr.read_element(vs2, ElemIdx::new(i), src_sew);

        let wide = match op {
            WidenIntReduceOp::SumU => elem,
            WidenIntReduceOp::Sum => src_sew.sign_extend(elem).cast_unsigned() & dst_mask,
        };

        acc = acc.wrapping_add(wide) & dst_mask;
    }

    vpr.write_element(vd, ElemIdx::new(0), dst_sew, acc);

    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), dst_sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.fill_agnostic_element(vd, ElemIdx::new(i), dst_sew);
        }
    }

    VecExecResult::default()
}

/// Execute a floating-point reduction at SEW width (32 or 64).
///
/// For ordered sums (`vfredosum`) elements are processed sequentially from
/// element 0 to `vl-1`. Unordered sums (`vfredusum`) use the same sequential
/// ordering for deterministic results.
///
/// FP min/max reductions (`vfredmin`, `vfredmax`) use IEEE 754-2008 minNum/maxNum
/// semantics, matching the scalar `fmin`/`fmax` helpers.
fn exec_fp_reduction(
    op: FpReduceOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let sew = ctx.sew;
    let Some(fp_sew) = FpSew::of(sew, ctx.zvfh) else {
        return VecExecResult::default();
    };

    let saved_rm = set_host_round_mode(ctx.frm);
    let (result_bits, flags) = match fp_sew {
        FpSew::F16 => fp_reduce::<SoftF16>(op, vpr, vs2, vs1, ctx),
        FpSew::F32 => fp_reduce::<f32>(op, vpr, vs2, vs1, ctx),
        FpSew::F64 => fp_reduce::<f64>(op, vpr, vs2, vs1, ctx),
    };
    vpr.write_element(vd, ElemIdx::new(0), sew, result_bits);
    restore_host_round_mode(saved_rm);

    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.fill_agnostic_element(vd, ElemIdx::new(i), sew);
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// One floating-point reduction loop. Per V 1.0 §14.2 and Zvfh, each step
/// rounds to SEW.
///
/// Returns `(result_bits, fp_flags)`, the result as an element at SEW.
fn fp_reduce<F: VecFloat>(
    op: FpReduceOp,
    vpr: &impl VectorRegFile,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> (u64, FpFlags) {
    let mut acc = F::from_raw_bits(read_initial_accum(vpr, vs1, F::SEW));
    let mut flags = FpFlags::NONE;

    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem = F::from_raw_bits(vpr.read_element(vs2, ElemIdx::new(i), F::SEW));
        let (next, f) = match op {
            FpReduceOp::OSum | FpReduceOp::USum => acc.add(elem, ctx.frm),
            FpReduceOp::Min => (acc.min(elem), invalid_if_signaling(acc, elem)),
            FpReduceOp::Max => (acc.max(elem), invalid_if_signaling(acc, elem)),
        };
        acc = next;
        flags = flags | f;
    }

    (acc.canonical().raw_bits(), flags)
}

/// Execute a widening FP reduction.
///
/// Source elements are at SEW; accumulator and result are at 2*SEW. Per
/// V 1.0 §14.4: each step rounds the running sum to the destination
/// precision. Supported widenings: E32→E64, and E16→E32 when Zvfh is
/// enabled.
fn exec_fp_widen_reduction(
    op: FpWidenReduceOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(widen) = FpSew::of(ctx.sew, ctx.zvfh).and_then(FpSew::widening) else {
        return VecExecResult::default();
    };
    let dst_sew = widen.dst();
    let saved_rm = set_host_round_mode(ctx.frm);

    let (result_bits, flags) = match widen {
        FpWiden::F32ToF64 => fp_widen_reduce::<f32, f64>(op, vpr, vs2, vs1, ctx),
        FpWiden::F16ToF32 => fp_widen_reduce::<SoftF16, f32>(op, vpr, vs2, vs1, ctx),
    };

    vpr.write_element(vd, ElemIdx::new(0), dst_sew, result_bits);

    restore_host_round_mode(saved_rm);

    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), dst_sew, Vlmul::M1).as_usize();
        for i in 1..vlmax {
            vpr.fill_agnostic_element(vd, ElemIdx::new(i), dst_sew);
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// One widening floating-point reduction loop: `N` elements summed into a
/// `W` accumulator, each step rounded to `W`.
fn fp_widen_reduce<N: VecFloat, W: VecFloat>(
    op: FpWidenReduceOp,
    vpr: &impl VectorRegFile,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> (u64, FpFlags) {
    let mut acc = W::from_raw_bits(read_initial_accum(vpr, vs1, W::SEW));
    let mut flags = FpFlags::NONE;
    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        let elem = N::from_raw_bits(vpr.read_element(vs2, ElemIdx::new(i), N::SEW));
        match op {
            FpWidenReduceOp::OSum | FpWidenReduceOp::USum => {
                if elem.is_signaling_nan() {
                    flags = flags | FpFlags::NV;
                }
                let (sum, f) = acc.add(W::from_f64_exact(elem.to_f64()), ctx.frm);
                acc = sum;
                flags = flags | f;
            }
        }
    }
    (acc.canonical().raw_bits(), flags)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    use crate::arch::regs::vpr::Vpr;
    use crate::isa::fp::RoundingMode;
    use crate::isa::rvv::{MaskPolicy, TailPolicy, Vlen, Vlmul, Vxrm};

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
            frm: RoundingMode::Rne,
            zvfh: false,
        }
    }

    /// Create a 128-bit VLEN vector register file.
    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    #[test]
    fn vredsum_adds_every_element_to_the_accumulator() {
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

        let _result = vec_reduce(ReduceOp::Int(IntReduceOp::Sum), &mut vpr, vd, vs2, vs1, &ctx);
        // 100 + 10 + 20 + 30 + 40 = 200
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 200);
    }

    #[test]
    fn vredand_ands_every_element_into_the_accumulator() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        for i in 0..4 {
            vpr.write_element(vs2, ElemIdx::new(i), Sew::E32, 0xFF);
        }
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, 0xFFFF_FFFF);

        let _result = vec_reduce(ReduceOp::Int(IntReduceOp::And), &mut vpr, vd, vs2, vs1, &ctx);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0xFF);
    }

    #[test]
    fn vredmin_takes_the_signed_minimum_of_elements_and_accumulator() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        // vs2 = [5, -3, 10, 1] as signed i32
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 5);
        vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, u64::from((-3i32).cast_unsigned()));
        vpr.write_element(vs2, ElemIdx::new(2), Sew::E32, 10);
        vpr.write_element(vs2, ElemIdx::new(3), Sew::E32, 1);
        // accumulator = 100
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, 100);

        let _result = vec_reduce(ReduceOp::Int(IntReduceOp::Min), &mut vpr, vd, vs2, vs1, &ctx);
        // min(100, 5, -3, 10, 1) = -3
        let val = vpr.read_element(vd, ElemIdx::new(0), Sew::E32);
        assert_eq!(val as u32, (-3i32).cast_unsigned());
    }

    #[test]
    fn vfredosum_adds_single_precision_elements_to_the_accumulator() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let vd = VRegIdx::new(1);
        let vs2 = VRegIdx::new(2);
        let vs1 = VRegIdx::new(3);

        // vs2 = [1.0, 2.0, 3.0, 4.0], accum = 10.0
        for i in 0..4 {
            vpr.write_element(
                vs2,
                ElemIdx::new(i),
                Sew::E32,
                u64::from((i as f32 + 1.0).to_bits()),
            );
        }
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E32, u64::from(10.0f32.to_bits()));

        let _result = vec_reduce(ReduceOp::Fp(FpReduceOp::OSum), &mut vpr, vd, vs2, vs1, &ctx);
        let val = f32::from_bits(vpr.read_element(vd, ElemIdx::new(0), Sew::E32) as u32);
        assert_eq!(val, 20.0); // 10 + 1 + 2 + 3 + 4
    }

    #[test]
    fn a_widening_sum_of_a_single_precision_signaling_nan_is_invalid() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 1);
        let (vd, vs2, vs1) = (VRegIdx::new(1), VRegIdx::new(2), VRegIdx::new(4));
        vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0x7fa0_0000);
        vpr.write_element(vs1, ElemIdx::new(0), Sew::E64, 1f64.to_bits());

        let result =
            vec_reduce(ReduceOp::FpWiden(FpWidenReduceOp::USum), &mut vpr, vd, vs2, vs1, &ctx);

        assert!(result.fp_flags.contains(FpFlags::NV));
    }
}

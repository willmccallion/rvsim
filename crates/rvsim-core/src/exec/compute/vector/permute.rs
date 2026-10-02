//! Vector permutation operations.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) permutation operations:
//! scalar moves (`vmv.x.s`, `vmv.s.x`), slides (`vslideup`, `vslidedown`,
//! `vslide1up`, `vslide1down`), gathers (`vrgather`, `vrgatherei16`),
//! compress (`vcompress`), and whole-register moves (`vmv<n>r`).
//!
//! The main entry point [`vec_permute_execute`] dispatches to the appropriate
//! operation based on the [`VectorOp`] variant.

use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand, mask_active};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::{PermuteOp, SlideOffset};
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};

/// The sources a permutation may draw on: `operand1` as the instruction
/// encodes it, the `vs1` register, and the value of `rs1`.
#[derive(Clone, Copy, Debug)]
pub struct PermuteSources {
    /// The first operand as encoded (vector, scalar or immediate).
    pub operand1: VecOperand,
    /// The `vs1` register, for the forms that only take a vector there.
    pub vs1: VRegIdx,
    /// The value of `rs1`, for the forms that only take a scalar.
    pub rs1: u64,
}

/// Execute a permutation operation.
///
/// For scalar-producing ops (`vmv.x.s`), the scalar value is returned in
/// [`VecExecResult::scalar_result`]. For vector-producing ops, results are
/// written directly to `vd` in the VPR.
pub fn vec_permute_execute(
    op: PermuteOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    sources: PermuteSources,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        PermuteOp::MvXS => exec_vmv_xs(vpr, vs2, ctx),
        PermuteOp::MvSX => exec_vmv_sx(vpr, vd, sources.rs1, ctx),
        PermuteOp::SlideUp(offset) => {
            exec_slideup(vpr, vd, vs2, slide_offset(offset, sources.rs1), ctx)
        }
        PermuteOp::SlideDown(offset) => {
            exec_slidedown(vpr, vd, vs2, slide_offset(offset, sources.rs1), ctx)
        }
        PermuteOp::Slide1Up => exec_slide1up(vpr, vd, vs2, sources.rs1, ctx),
        PermuteOp::Slide1Down => exec_slide1down(vpr, vd, vs2, sources.rs1, ctx),
        PermuteOp::Rgather => exec_rgather(vpr, vd, vs2, &sources.operand1, ctx),
        PermuteOp::RgatherEi16 => exec_rgather_ei16(vpr, vd, vs2, sources.vs1, ctx),
        PermuteOp::Compress => exec_compress(vpr, vd, vs2, sources.vs1, ctx),
        PermuteOp::WholeMove(nregs) => exec_whole_reg_move(vpr, vd, vs2, nregs),
    }
}

/// The element offset of a slide.
const fn slide_offset(offset: SlideOffset, rs1: u64) -> usize {
    match offset {
        SlideOffset::Rs1 => rs1 as usize,
        SlideOffset::Imm(imm) => imm as usize,
    }
}

/// Build a default result with no flags set.
#[inline]
const fn no_flags_result(scalar: Option<u64>) -> VecExecResult {
    VecExecResult { vxsat: false, scalar_result: scalar, fp_flags: FpFlags::NONE }
}

/// `vmv.x.s` — move vs2[0] to a scalar GPR result.
///
/// Reads element 0 of `vs2` at the current SEW and returns it as the scalar
/// result.  Does not write any vector register.
fn exec_vmv_xs(vpr: &impl VectorRegFile, vs2: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    let val = vpr.read_element(vs2, ElemIdx::new(0), ctx.sew);
    // RVV 1.0: vmv.x.s sign-extends the SEW-width value to XLEN.
    let sign_extended = ctx.sew.sign_extend(val) as u64;
    no_flags_result(Some(sign_extended))
}

/// `vmv.s.x` — move a scalar GPR value into vd[0].
///
/// Writes the scalar to element 0 of `vd` at the current SEW.  Remaining
/// elements (indices 1..vlmax) follow the tail policy.
fn exec_vmv_sx(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    rs1: u64,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let scalar = rs1 & ctx.sew.mask();

    if ctx.vl > 0 {
        vpr.write_element(vd, ElemIdx::new(0), ctx.sew, scalar);
    }

    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
        for i in 1..vlmax {
            vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
        }
    }

    no_flags_result(None)
}

/// `vslideup` — slide elements up by a given offset.
///
/// For each element i in [vstart, vl):
/// - If i < offset: element is unchanged (left undisturbed).
/// - If i >= offset: vd[i] = vs2[i - offset].
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slideup(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    offset: usize,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if i >= offset {
            let src_idx = i - offset;
            let val = vpr.read_element(vs2, ElemIdx::new(src_idx), ctx.sew);
            vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
        }
    }

    no_flags_result(None)
}

/// `vslidedown` — slide elements down by a given offset.
///
/// For each element i in [vstart, vl):
/// - If (i + offset) < vlmax: vd[i] = vs2[i + offset].
/// - Otherwise: vd[i] = 0.
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slidedown(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    offset: usize,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        // Saturating add prevents wrap when offset is huge (e.g. rs1 = -1
        // sign-extended to 0xFFFF…FFFF treated as unsigned per spec §16.4).
        let src_idx = i.saturating_add(offset);
        let val =
            if src_idx < vlmax { vpr.read_element(vs2, ElemIdx::new(src_idx), ctx.sew) } else { 0 };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vslide1up` — slide up by one, inserting a scalar at element 0.
///
/// - vd[0] = scalar from operand1 (rs1).
/// - vd[i] = vs2[i - 1] for i in 1..vl.
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slide1up(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    rs1: u64,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = rs1 & ctx.sew.mask();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let val = if i == 0 { scalar } else { vpr.read_element(vs2, ElemIdx::new(i - 1), ctx.sew) };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vslide1down` — slide down by one, inserting a scalar at the last active
/// element.
///
/// - vd[i] = vs2[i + 1] for i in 0..vl-1.
/// - vd[vl - 1] = scalar from operand1 (rs1).
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slide1down(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    rs1: u64,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = rs1 & ctx.sew.mask();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let val = if i == ctx.vl - 1 {
            scalar
        } else {
            vpr.read_element(vs2, ElemIdx::new(i + 1), ctx.sew)
        };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vrgather` — register gather (permute by index).
///
/// For each active element i in [vstart, vl):
/// - Read the index from operand1 (vector, scalar, or immediate).
/// - If index >= vlmax, write 0.
/// - Otherwise, write vs2[index].
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_rgather(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let index = match operand1 {
            VecOperand::Vector(vs1) => vpr.read_element(*vs1, ElemIdx::new(i), ctx.sew) as usize,
            VecOperand::Scalar(v) => *v as usize,
            VecOperand::Immediate(v) => *v as u64 as usize,
        };

        let val =
            if index >= vlmax { 0 } else { vpr.read_element(vs2, ElemIdx::new(index), ctx.sew) };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vrgatherei16` — register gather with 16-bit index vector.
///
/// Same as [`exec_rgather`] but indices from vs1 are always read at
/// [`Sew::E16`] regardless of the current SEW.  Data from vs2 is read at the
/// current SEW.
fn exec_rgather_ei16(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        // Indices are always at EEW=16, regardless of current SEW.
        let index = vpr.read_element(vs1, ElemIdx::new(i), Sew::E16) as usize;

        let val =
            if index >= vlmax { 0 } else { vpr.read_element(vs2, ElemIdx::new(index), ctx.sew) };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vcompress` — compress active elements from vs2 into vd.
///
/// Scans vs2 using the vs1 mask: elements where the corresponding vs1 bit is
/// set are packed contiguously into vd starting from element 0.  The operation
/// is always unmasked (vm=1); the mask register vs1 selects which source
/// elements to include, not which destination elements to write.
///
/// Tail elements (after the last compressed element) follow the tail policy.
fn exec_compress(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let mut dst = 0usize;

    // Phase 1: pack active elements selected by vs1 mask.
    for i in ctx.vstart..ctx.vl {
        if vpr.read_mask_bit(vs1, ElemIdx::new(i)) {
            let val = vpr.read_element(vs2, ElemIdx::new(i), ctx.sew);
            vpr.write_element(vd, ElemIdx::new(dst), ctx.sew, val);
            dst += 1;
        }
    }

    // Phase 2: tail policy — write all-1s for remaining elements if agnostic.
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    if ctx.vta.is_agnostic() {
        for i in dst..vlmax {
            vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
        }
    }

    no_flags_result(None)
}

/// `vmv<n>r` — whole-register move of `n` consecutive registers.
///
/// Copies raw register bytes from vs2..vs2+(n-1) to vd..vd+(n-1).  Ignores
/// `vl`, `vtype`, masking, and tail policy — this is a raw byte copy.
fn exec_whole_reg_move(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    nregs: u8,
) -> VecExecResult {
    for offset in 0..nregs {
        let src = VRegIdx::new(vs2.as_u8() + offset);
        let dst = VRegIdx::new(vd.as_u8() + offset);
        vpr.copy_reg(dst, src);
    }
    no_flags_result(None)
}

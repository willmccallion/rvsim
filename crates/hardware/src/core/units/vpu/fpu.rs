//! Vector Floating-Point Unit.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) floating-point operations.
//! The main entry point [`vec_fp_execute`] dispatches to per-element loops that
//! handle masking, prestart/tail policy, NaN boxing, and FP exception flag
//! accumulation.
//!
//! Operations are grouped into categories:
//! - Arithmetic: add, sub, mul, div, sqrt, rsub, rdiv
//! - Min/max: IEEE 754-2008 minNum/maxNum
//! - Sign injection: sgnj, sgnjn, sgnjx
//! - Fused multiply-add: fmacc, fnmacc, fmsac, fnmsac, fmadd, fnmadd, fmsub, fnmsub
//! - Comparison (write mask): feq, fne, flt, fle, fgt, fge
//! - Classification: vfclass
//! - Conversions: int<->float, widening, narrowing
//! - Merge/move: vfmerge, vfmv.s.f, vfmv.f.s
//! - Slide: vfslide1up, vfslide1down

#![allow(clippy::float_cmp)]

use crate::core::arch::vpr::Vpr;
use crate::core::pipeline::signals::VectorOp;
use crate::core::units::fpu::exception_flags::FpFlags;
use crate::core::units::fpu::nan_handling::{
    box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
};
use crate::core::units::fpu::{clear_host_fp_flags, read_host_fp_flags};
use crate::core::units::vpu::alu::{VecExecCtx, VecExecResult, VecOperand};
use crate::core::units::vpu::types::{ElemIdx, MaskPolicy, Sew, TailPolicy, VRegIdx, Vlmax};

// ============================================================================
// Public helpers
// ============================================================================

/// Returns `true` if `op` is a vector floating-point operation handled by this module.
pub const fn is_vec_fp(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFAdd
            | VectorOp::VFSub
            | VectorOp::VFRSub
            | VectorOp::VFMul
            | VectorOp::VFDiv
            | VectorOp::VFRDiv
            | VectorOp::VFMin
            | VectorOp::VFMax
            | VectorOp::VFSgnj
            | VectorOp::VFSgnjn
            | VectorOp::VFSgnjx
            | VectorOp::VMFEq
            | VectorOp::VMFNe
            | VectorOp::VMFLt
            | VectorOp::VMFLe
            | VectorOp::VMFGt
            | VectorOp::VMFGe
            | VectorOp::VFMacc
            | VectorOp::VFNMacc
            | VectorOp::VFMSac
            | VectorOp::VFNMSac
            | VectorOp::VFMAdd
            | VectorOp::VFNMAdd
            | VectorOp::VFMSub
            | VectorOp::VFNMSub
            | VectorOp::VFSqrt
            | VectorOp::VFRsqrt7
            | VectorOp::VFRec7
            | VectorOp::VFClass
            | VectorOp::VFCvtXuF
            | VectorOp::VFCvtXF
            | VectorOp::VFCvtFXu
            | VectorOp::VFCvtFX
            | VectorOp::VFCvtRtzXuF
            | VectorOp::VFCvtRtzXF
            | VectorOp::VFWAdd
            | VectorOp::VFWSub
            | VectorOp::VFWMul
            | VectorOp::VFWAddW
            | VectorOp::VFWSubW
            | VectorOp::VFWMacc
            | VectorOp::VFWNMacc
            | VectorOp::VFWMSac
            | VectorOp::VFWNMSac
            | VectorOp::VFWCvtXuF
            | VectorOp::VFWCvtXF
            | VectorOp::VFWCvtFXu
            | VectorOp::VFWCvtFX
            | VectorOp::VFWCvtFF
            | VectorOp::VFWCvtRtzXuF
            | VectorOp::VFWCvtRtzXF
            | VectorOp::VFNCvtXuF
            | VectorOp::VFNCvtXF
            | VectorOp::VFNCvtFXu
            | VectorOp::VFNCvtFX
            | VectorOp::VFNCvtFF
            | VectorOp::VFNCvtRodFF
            | VectorOp::VFNCvtRtzXuF
            | VectorOp::VFNCvtRtzXF
            | VectorOp::VFMerge
            | VectorOp::VFMvSF
            | VectorOp::VFMvFS
            | VectorOp::VFSlide1Up
            | VectorOp::VFSlide1Down
    )
}

// ============================================================================
// Entry point
// ============================================================================

/// Execute a vector floating-point operation.
///
/// This is the main entry point for all vector FP operations. It dispatches to
/// specialised loops based on the operation category.
///
/// # Arguments
///
/// * `op`       - The vector floating-point operation to perform.
/// * `vpr`      - Mutable reference to the vector register file.
/// * `vd_idx`   - Destination vector register index.
/// * `vs2_idx`  - Second source vector register index.
/// * `operand1` - First operand (vector, scalar, or immediate).
/// * `ctx`      - Execution context (SEW, vl, masking policies, etc.).
#[allow(clippy::too_many_lines)]
pub fn vec_fp_execute(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Scalar move: vfmv.f.s — read vs2[0] as scalar result
    if op == VectorOp::VFMvFS {
        let val = vpr.read_element(vs2_idx, ElemIdx::new(0), ctx.sew);
        return VecExecResult { vxsat: false, scalar_result: Some(val), fp_flags: FpFlags::NONE };
    }

    // Scalar move: vfmv.s.f — write scalar into vd[0]
    if op == VectorOp::VFMvSF {
        let scalar = match operand1 {
            VecOperand::Scalar(s) => s,
            _ => 0,
        };
        if ctx.vl > 0 {
            vpr.write_element(vd_idx, ElemIdx::new(0), ctx.sew, scalar & ctx.sew.mask());
        }
        // Tail elements
        let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
        for i in 1..vlmax {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd_idx, i, ctx.sew);
            }
        }
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    }

    // Merge
    if op == VectorOp::VFMerge {
        return exec_fp_merge(vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Slide operations
    if matches!(op, VectorOp::VFSlide1Up | VectorOp::VFSlide1Down) {
        return exec_fp_slide1(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Comparisons (write mask bits)
    if is_fp_comparison(op) {
        return exec_fp_comparison(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Widening operations
    if is_fp_widening(op) {
        return exec_fp_widening(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Narrowing operations
    if is_fp_narrowing(op) {
        return exec_fp_narrowing(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // FMA operations (need vd as accumulator)
    if is_fp_fma(op) {
        return exec_fp_fma(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Standard element-wise FP operations
    exec_fp_standard(op, vpr, vd_idx, vs2_idx, operand1, ctx)
}

// ============================================================================
// Internal helpers
// ============================================================================

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

/// Read operand1 value for element `i` at the given SEW.
#[inline]
fn read_op1(vpr: &Vpr, operand1: &VecOperand, i: usize, sew: Sew) -> u64 {
    match operand1 {
        VecOperand::Vector(vs1) => vpr.read_element(*vs1, ElemIdx::new(i), sew),
        VecOperand::Scalar(s) => *s & sew.mask(),
        VecOperand::Immediate(imm) => (*imm as u64) & sew.mask(),
    }
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

/// Convert a raw u64 element to f32 (unboxed from lower 32 bits).
#[inline]
const fn elem_to_f32(val: u64) -> f32 {
    f32::from_bits(val as u32)
}

/// Convert a raw u64 element to f64.
#[inline]
const fn elem_to_f64(val: u64) -> f64 {
    f64::from_bits(val)
}

/// RISC-V FCLASS for f32: returns 10-bit classification bitmask.
const fn classify_f32(val: u32) -> u64 {
    let sign = (val >> 31) & 1;
    let exp = (val >> 23) & 0xFF;
    let frac = val & 0x007F_FFFF;

    if exp == 0xFF && frac != 0 {
        // NaN
        if frac & 0x0040_0000 != 0 {
            1 << 9 // qNaN
        } else {
            1 << 8 // sNaN
        }
    } else if exp == 0xFF {
        if sign != 0 { 1 << 0 } else { 1 << 7 } // ±inf
    } else if exp == 0 && frac == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 } // ±zero
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 } // ±subnormal
    } else if sign != 0 {
        1 << 1 // negative normal
    } else {
        1 << 6 // positive normal
    }
}

/// RISC-V FCLASS for f64: returns 10-bit classification bitmask.
const fn classify_f64(val: u64) -> u64 {
    let sign = (val >> 63) & 1;
    let exp = (val >> 52) & 0x7FF;
    let frac = val & 0x000F_FFFF_FFFF_FFFF;

    if exp == 0x7FF && frac != 0 {
        if frac & 0x0008_0000_0000_0000 != 0 {
            1 << 9 // qNaN
        } else {
            1 << 8 // sNaN
        }
    } else if exp == 0x7FF {
        if sign != 0 { 1 << 0 } else { 1 << 7 } // ±inf
    } else if exp == 0 && frac == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 } // ±zero
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 } // ±subnormal
    } else if sign != 0 {
        1 << 1
    } else {
        1 << 6
    }
}

/// Bit mask for the sign bit in a 32-bit IEEE 754 float.
const F32_SIGN_BIT: u32 = 0x8000_0000;

/// Bit mask for the sign bit in a 64-bit IEEE 754 float.
const F64_SIGN_BIT: u64 = 0x8000_0000_0000_0000;

/// Returns `true` for FP comparison ops that write mask bits.
const fn is_fp_comparison(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VMFEq
            | VectorOp::VMFNe
            | VectorOp::VMFLt
            | VectorOp::VMFLe
            | VectorOp::VMFGt
            | VectorOp::VMFGe
    )
}

/// Returns `true` for FP widening operations.
const fn is_fp_widening(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFWAdd
            | VectorOp::VFWSub
            | VectorOp::VFWMul
            | VectorOp::VFWAddW
            | VectorOp::VFWSubW
            | VectorOp::VFWMacc
            | VectorOp::VFWNMacc
            | VectorOp::VFWMSac
            | VectorOp::VFWNMSac
            | VectorOp::VFWCvtXuF
            | VectorOp::VFWCvtXF
            | VectorOp::VFWCvtFXu
            | VectorOp::VFWCvtFX
            | VectorOp::VFWCvtFF
            | VectorOp::VFWCvtRtzXuF
            | VectorOp::VFWCvtRtzXF
    )
}

/// Returns `true` for FP narrowing operations.
const fn is_fp_narrowing(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFNCvtXuF
            | VectorOp::VFNCvtXF
            | VectorOp::VFNCvtFXu
            | VectorOp::VFNCvtFX
            | VectorOp::VFNCvtFF
            | VectorOp::VFNCvtRodFF
            | VectorOp::VFNCvtRtzXuF
            | VectorOp::VFNCvtRtzXF
    )
}

/// Returns `true` for FMA operations (need vd as accumulator).
const fn is_fp_fma(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFMacc
            | VectorOp::VFNMacc
            | VectorOp::VFMSac
            | VectorOp::VFNMSac
            | VectorOp::VFMAdd
            | VectorOp::VFNMAdd
            | VectorOp::VFMSub
            | VectorOp::VFNMSub
    )
}

// ============================================================================
// Per-element FP computation (SEW=32)
// ============================================================================

/// Compute one standard FP element at SEW=32.
///
/// Returns `(result_bits, flags)`.
#[allow(clippy::too_many_lines)]
fn compute_f32(op: VectorOp, vs2_bits: u64, op1_bits: u64) -> (u64, FpFlags) {
    let a = elem_to_f32(vs2_bits);
    let b = elem_to_f32(op1_bits);

    match op {
        VectorOp::VFAdd => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) + std::hint::black_box(b));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFSub => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) - std::hint::black_box(b));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFRSub => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(b) - std::hint::black_box(a));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFMul => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFDiv => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFRDiv => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(b) / std::hint::black_box(a));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFSqrt => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a).sqrt());
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFRsqrt7 => {
            // Approximation: full precision 1/sqrt for simulator
            clear_host_fp_flags();
            let r = std::hint::black_box(1.0f32 / std::hint::black_box(a).sqrt());
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFRec7 => {
            // Approximation: full precision 1/x for simulator
            clear_host_fp_flags();
            let r = std::hint::black_box(1.0f32 / std::hint::black_box(a));
            (box_f32_canon(r), read_host_fp_flags())
        }
        VectorOp::VFMin => {
            let r = fmin_f32(a, b);
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFMax => {
            let r = fmax_f32(a, b);
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFSgnj => {
            let r = f32::from_bits((a.to_bits() & !F32_SIGN_BIT) | (b.to_bits() & F32_SIGN_BIT));
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFSgnjn => {
            let r = f32::from_bits((a.to_bits() & !F32_SIGN_BIT) | (!b.to_bits() & F32_SIGN_BIT));
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFSgnjx => {
            let r = f32::from_bits(a.to_bits() ^ (b.to_bits() & F32_SIGN_BIT));
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFClass => (classify_f32(vs2_bits as u32), FpFlags::NONE),
        // Conversions: float -> unsigned int
        VectorOp::VFCvtXuF => {
            clear_host_fp_flags();
            let r = if a.is_nan() {
                u32::MAX as u64
            } else {
                std::hint::black_box(std::hint::black_box(a) as u32) as u64
            };
            (r, read_host_fp_flags())
        }
        // Conversions: float -> signed int
        VectorOp::VFCvtXF => {
            clear_host_fp_flags();
            let r = if a.is_nan() {
                i32::MAX as u64
            } else {
                std::hint::black_box(std::hint::black_box(a) as i32) as u64
            };
            (r & 0xFFFF_FFFF, read_host_fp_flags())
        }
        // Conversions with RTZ: float -> unsigned int
        VectorOp::VFCvtRtzXuF => {
            let r = if a.is_nan() { u32::MAX as u64 } else { a.trunc() as u32 as u64 };
            (r, FpFlags::NONE)
        }
        // Conversions with RTZ: float -> signed int
        VectorOp::VFCvtRtzXF => {
            let r =
                if a.is_nan() { i32::MAX as u32 as u64 } else { a.trunc() as i32 as u32 as u64 };
            (r, FpFlags::NONE)
        }
        // Conversions: unsigned int -> float
        VectorOp::VFCvtFXu => {
            clear_host_fp_flags();
            let r = std::hint::black_box(vs2_bits as u32 as f32);
            (box_f32_canon(r), read_host_fp_flags())
        }
        // Conversions: signed int -> float
        VectorOp::VFCvtFX => {
            clear_host_fp_flags();
            let r = std::hint::black_box(sign_extend(vs2_bits, Sew::E32) as i32 as f32);
            (box_f32_canon(r), read_host_fp_flags())
        }
        _ => (0, FpFlags::NONE),
    }
}

// ============================================================================
// Per-element FP computation (SEW=64)
// ============================================================================

/// Compute one standard FP element at SEW=64.
///
/// Returns `(result_bits, flags)`.
#[allow(clippy::too_many_lines)]
fn compute_f64(op: VectorOp, vs2_bits: u64, op1_bits: u64) -> (u64, FpFlags) {
    let a = elem_to_f64(vs2_bits);
    let b = elem_to_f64(op1_bits);

    match op {
        VectorOp::VFAdd => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) + std::hint::black_box(b));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFSub => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) - std::hint::black_box(b));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFRSub => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(b) - std::hint::black_box(a));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFMul => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFDiv => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFRDiv => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(b) / std::hint::black_box(a));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFSqrt => {
            clear_host_fp_flags();
            let r = std::hint::black_box(std::hint::black_box(a).sqrt());
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFRsqrt7 => {
            clear_host_fp_flags();
            let r = std::hint::black_box(1.0f64 / std::hint::black_box(a).sqrt());
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFRec7 => {
            clear_host_fp_flags();
            let r = std::hint::black_box(1.0f64 / std::hint::black_box(a));
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFMin => {
            let r = fmin_f64(a, b);
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFMax => {
            let r = fmax_f64(a, b);
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFSgnj => {
            let r = f64::from_bits((a.to_bits() & !F64_SIGN_BIT) | (b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFSgnjn => {
            let r = f64::from_bits((a.to_bits() & !F64_SIGN_BIT) | (!b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFSgnjx => {
            let r = f64::from_bits(a.to_bits() ^ (b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFClass => (classify_f64(vs2_bits), FpFlags::NONE),
        VectorOp::VFCvtXuF => {
            clear_host_fp_flags();
            let r = if a.is_nan() {
                u64::MAX
            } else {
                std::hint::black_box(std::hint::black_box(a) as u64)
            };
            (r, read_host_fp_flags())
        }
        VectorOp::VFCvtXF => {
            clear_host_fp_flags();
            let r = if a.is_nan() {
                i64::MAX as u64
            } else {
                std::hint::black_box(std::hint::black_box(a) as i64) as u64
            };
            (r, read_host_fp_flags())
        }
        VectorOp::VFCvtRtzXuF => {
            let r = if a.is_nan() { u64::MAX } else { a.trunc() as u64 };
            (r, FpFlags::NONE)
        }
        VectorOp::VFCvtRtzXF => {
            let r = if a.is_nan() { i64::MAX as u64 } else { a.trunc() as i64 as u64 };
            (r, FpFlags::NONE)
        }
        VectorOp::VFCvtFXu => {
            clear_host_fp_flags();
            let r = std::hint::black_box(vs2_bits as f64);
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        VectorOp::VFCvtFX => {
            clear_host_fp_flags();
            let r = std::hint::black_box(vs2_bits as i64 as f64);
            (canonicalize_f64_bits(r), read_host_fp_flags())
        }
        _ => (0, FpFlags::NONE),
    }
}

// ============================================================================
// Standard element-wise loop
// ============================================================================

/// Standard (non-widening, non-narrowing, non-FMA) FP element-wise loop.
fn exec_fp_standard(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

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

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_f32(op, vs2_val, op1_val),
            Sew::E64 => compute_f64(op, vs2_val, op1_val),
            _ => (0, FpFlags::NONE), // FP not supported at E8/E16
        };
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

// ============================================================================
// FMA loop
// ============================================================================

/// FMA operations: vd is both source (accumulator) and destination.
fn exec_fp_fma(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

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

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_fma_f32(op, vs2_val, op1_val, vd_val),
            Sew::E64 => compute_fma_f64(op, vs2_val, op1_val, vd_val),
            _ => (0, FpFlags::NONE),
        };
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Compute FMA for f32 element.
///
/// RVV FMA conventions:
/// - `vfmacc.vv`: vd[i] = vs1[i]*vs2[i] + vd[i]
/// - `vfnmacc.vv`: vd[i] = -(vs1[i]*vs2[i]) - vd[i]
/// - `vfmsac.vv`: vd[i] = vs1[i]*vs2[i] - vd[i]
/// - `vfnmsac.vv`: vd[i] = -(vs1[i]*vs2[i]) + vd[i]
/// - `vfmadd.vv`: vd[i] = vs1[i]*vd[i] + vs2[i]
/// - `vfnmadd.vv`: vd[i] = -(vs1[i]*vd[i]) - vs2[i]
/// - `vfmsub.vv`: vd[i] = vs1[i]*vd[i] - vs2[i]
/// - `vfnmsub.vv`: vd[i] = -(vs1[i]*vd[i]) + vs2[i]
fn compute_fma_f32(op: VectorOp, vs2_bits: u64, op1_bits: u64, vd_bits: u64) -> (u64, FpFlags) {
    let vs2 = elem_to_f32(vs2_bits);
    let op1 = elem_to_f32(op1_bits);
    let vd = elem_to_f32(vd_bits);

    clear_host_fp_flags();
    let r =
        std::hint::black_box(match op {
            VectorOp::VFMacc => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFNMacc => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFMSac => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFNMSac => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFMAdd => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            VectorOp::VFNMAdd => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFMSub => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFNMSub => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            _ => 0.0,
        });
    (box_f32_canon(r), read_host_fp_flags())
}

/// Compute FMA for f64 element.
fn compute_fma_f64(op: VectorOp, vs2_bits: u64, op1_bits: u64, vd_bits: u64) -> (u64, FpFlags) {
    let vs2 = elem_to_f64(vs2_bits);
    let op1 = elem_to_f64(op1_bits);
    let vd = elem_to_f64(vd_bits);

    clear_host_fp_flags();
    let r =
        std::hint::black_box(match op {
            VectorOp::VFMacc => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFNMacc => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFMSac => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFNMSac => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFMAdd => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            VectorOp::VFNMAdd => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFMSub => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFNMSub => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            _ => 0.0,
        });
    (canonicalize_f64_bits(r), read_host_fp_flags())
}

// ============================================================================
// Comparison loop
// ============================================================================

/// FP comparison loop: writes mask bits to vd.
fn exec_fp_comparison(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

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

        let (result, f) = match ctx.sew {
            Sew::E32 => {
                let a = elem_to_f32(vs2_val);
                let b = elem_to_f32(op1_val);
                // NaN comparisons: FLT/FLE/FGT/FGE raise NV on any NaN;
                // FEQ/FNE raise NV only on sNaN
                let nv = match op {
                    VectorOp::VMFEq | VectorOp::VMFNe => is_snan_f32(a) || is_snan_f32(b),
                    _ => a.is_nan() || b.is_nan(),
                };
                let f = if nv { FpFlags::NV } else { FpFlags::NONE };
                let cmp = match op {
                    VectorOp::VMFEq => a == b,
                    VectorOp::VMFNe => a != b,
                    VectorOp::VMFLt => a < b,
                    VectorOp::VMFLe => a <= b,
                    VectorOp::VMFGt => a > b,
                    VectorOp::VMFGe => a >= b,
                    _ => false,
                };
                (cmp, f)
            }
            Sew::E64 => {
                let a = elem_to_f64(vs2_val);
                let b = elem_to_f64(op1_val);
                let nv = match op {
                    VectorOp::VMFEq | VectorOp::VMFNe => is_snan_f64(a) || is_snan_f64(b),
                    _ => a.is_nan() || b.is_nan(),
                };
                let f = if nv { FpFlags::NV } else { FpFlags::NONE };
                let cmp = match op {
                    VectorOp::VMFEq => a == b,
                    VectorOp::VMFNe => a != b,
                    VectorOp::VMFLt => a < b,
                    VectorOp::VMFLe => a <= b,
                    VectorOp::VMFGt => a > b,
                    VectorOp::VMFGe => a >= b,
                    _ => false,
                };
                (cmp, f)
            }
            _ => (false, FpFlags::NONE),
        };

        flags = flags | f;
        vpr.write_mask_bit(vd_idx, ElemIdx::new(i), result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Checks if an f32 value is a signaling NaN.
const fn is_snan_f32(f: f32) -> bool {
    let bits = f.to_bits();
    let exp = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;
    let quiet_bit = bits & 0x0040_0000;
    exp == 0xFF && mantissa != 0 && quiet_bit == 0
}

/// Checks if an f64 value is a signaling NaN.
const fn is_snan_f64(f: f64) -> bool {
    let bits = f.to_bits();
    let exp = (bits >> 52) & 0x7FF;
    let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
    let quiet_bit = bits & 0x0008_0000_0000_0000;
    exp == 0x7FF && mantissa != 0 && quiet_bit == 0
}

// ============================================================================
// Merge loop
// ============================================================================

/// FP merge: like integer merge but for FP values.
fn exec_fp_merge(
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

        let use_op1 = ctx.vm || mask_active(vpr, i);
        let result = if use_op1 {
            read_op1(vpr, &operand1, i, ctx.sew)
        } else {
            vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew)
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}

// ============================================================================
// Slide1 loop
// ============================================================================

/// FP slide1up/slide1down operations.
fn exec_fp_slide1(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = match operand1 {
        VecOperand::Scalar(s) => s & ctx.sew.mask(),
        _ => 0,
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

        let result = match op {
            VectorOp::VFSlide1Up => {
                if i == 0 {
                    scalar
                } else {
                    vpr.read_element(vs2_idx, ElemIdx::new(i - 1), ctx.sew)
                }
            }
            VectorOp::VFSlide1Down => {
                if i == ctx.vl - 1 {
                    scalar
                } else {
                    vpr.read_element(vs2_idx, ElemIdx::new(i + 1), ctx.sew)
                }
            }
            _ => 0,
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}

// ============================================================================
// Widening FP loop
// ============================================================================

/// Widening FP operations: read at SEW, write at 2*SEW.
#[allow(clippy::too_many_lines)]
fn exec_fp_widening(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    // Determine if this is a ".w" variant (vs2 is already wide)
    let vs2_wide = matches!(op, VectorOp::VFWAddW | VectorOp::VFWSubW);

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

        // Read source elements
        let vs2_raw = if vs2_wide {
            vpr.read_element(vs2_idx, ElemIdx::new(i), wsew)
        } else {
            vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew)
        };
        let op1_raw = read_op1(vpr, &operand1, i, ctx.sew);

        // For widening FP: SEW=32 -> f32 inputs, f64 output
        if ctx.sew == Sew::E32 {
            // Handle integer-producing conversions (bypass the f64 path)
            if matches!(
                op,
                VectorOp::VFWCvtXuF
                    | VectorOp::VFWCvtXF
                    | VectorOp::VFWCvtRtzXuF
                    | VectorOp::VFWCvtRtzXF
                    | VectorOp::VFWCvtFXu
                    | VectorOp::VFWCvtFX
            ) {
                clear_host_fp_flags();
                let bits = match op {
                    VectorOp::VFWCvtXuF => {
                        let a = elem_to_f32(vs2_raw);
                        if a.is_nan() { u64::MAX } else { a as u64 }
                    }
                    VectorOp::VFWCvtXF => {
                        let a = elem_to_f32(vs2_raw);
                        if a.is_nan() { i64::MAX as u64 } else { a as i64 as u64 }
                    }
                    VectorOp::VFWCvtRtzXuF => {
                        let a = elem_to_f32(vs2_raw);
                        if a.is_nan() { u64::MAX } else { a.trunc() as u64 }
                    }
                    VectorOp::VFWCvtRtzXF => {
                        let a = elem_to_f32(vs2_raw);
                        if a.is_nan() { i64::MAX as u64 } else { a.trunc() as i64 as u64 }
                    }
                    VectorOp::VFWCvtFXu => (vs2_raw as u32 as f64).to_bits(),
                    VectorOp::VFWCvtFX => (sign_extend(vs2_raw, ctx.sew) as i32 as f64).to_bits(),
                    _ => 0,
                };
                let f = if matches!(op, VectorOp::VFWCvtRtzXuF | VectorOp::VFWCvtRtzXF) {
                    FpFlags::NONE
                } else {
                    read_host_fp_flags()
                };
                flags = flags | f;
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, bits);
                continue;
            }

            // FP arithmetic widening path
            let vs2_f = if vs2_wide { elem_to_f64(vs2_raw) } else { elem_to_f32(vs2_raw) as f64 };
            let op1_f = elem_to_f32(op1_raw) as f64;

            clear_host_fp_flags();
            let r = std::hint::black_box(match op {
                VectorOp::VFWAdd | VectorOp::VFWAddW => {
                    std::hint::black_box(vs2_f) + std::hint::black_box(op1_f)
                }
                VectorOp::VFWSub | VectorOp::VFWSubW => {
                    std::hint::black_box(vs2_f) - std::hint::black_box(op1_f)
                }
                VectorOp::VFWMul => std::hint::black_box(vs2_f) * std::hint::black_box(op1_f),
                _ => vs2_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, canonicalize_f64_bits(r));
        }
        // SEW=16 or others: not typically used for FP widening, skip
    }

    // Handle widening FMA separately in this same function
    if matches!(op, VectorOp::VFWMacc | VectorOp::VFWNMacc | VectorOp::VFWMSac | VectorOp::VFWNMSac)
    {
        return exec_fp_widening_fma(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Widening FMA operations.
fn exec_fp_widening_fma(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

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

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_raw = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_raw = vpr.read_element(vd_idx, ElemIdx::new(i), wsew);

        if ctx.sew == Sew::E32 {
            let vs2_f = elem_to_f32(vs2_raw) as f64;
            let op1_f = elem_to_f32(op1_raw) as f64;
            let vd_f = elem_to_f64(vd_raw);

            clear_host_fp_flags();
            let r = std::hint::black_box(match op {
                VectorOp::VFWMacc => std::hint::black_box(op1_f)
                    .mul_add(std::hint::black_box(vs2_f), std::hint::black_box(vd_f)),
                VectorOp::VFWNMacc => (-std::hint::black_box(op1_f))
                    .mul_add(std::hint::black_box(vs2_f), -std::hint::black_box(vd_f)),
                VectorOp::VFWMSac => std::hint::black_box(op1_f)
                    .mul_add(std::hint::black_box(vs2_f), -std::hint::black_box(vd_f)),
                VectorOp::VFWNMSac => (-std::hint::black_box(op1_f))
                    .mul_add(std::hint::black_box(vs2_f), std::hint::black_box(vd_f)),
                _ => vd_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, canonicalize_f64_bits(r));
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

// ============================================================================
// Narrowing FP loop
// ============================================================================

/// Narrowing FP operations: read at 2*SEW (or SEW for destination), write at SEW.
fn exec_fp_narrowing(
    op: VectorOp,
    vpr: &mut Vpr,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    _operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // For narrowing, the destination SEW is ctx.sew, source is 2*ctx.sew
    let Some(src_sew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

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

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);

        // Narrowing: src_sew=E64 -> dst_sew=E32
        let (result, f) = if ctx.sew == Sew::E32 {
            let a64 = elem_to_f64(vs2_raw);
            match op {
                VectorOp::VFNCvtFF | VectorOp::VFNCvtRodFF => {
                    clear_host_fp_flags();
                    let r = std::hint::black_box(std::hint::black_box(a64) as f32);
                    (box_f32_canon(r) & 0xFFFF_FFFF, read_host_fp_flags())
                }
                VectorOp::VFNCvtXuF => {
                    clear_host_fp_flags();
                    let r = if a64.is_nan() {
                        u32::MAX as u64
                    } else {
                        std::hint::black_box(a64) as u32 as u64
                    };
                    (r, read_host_fp_flags())
                }
                VectorOp::VFNCvtXF => {
                    clear_host_fp_flags();
                    let r = if a64.is_nan() {
                        i32::MAX as u32 as u64
                    } else {
                        std::hint::black_box(a64) as i32 as u32 as u64
                    };
                    (r, read_host_fp_flags())
                }
                VectorOp::VFNCvtRtzXuF => {
                    let r = if a64.is_nan() { u32::MAX as u64 } else { a64.trunc() as u32 as u64 };
                    (r, FpFlags::NONE)
                }
                VectorOp::VFNCvtRtzXF => {
                    let r = if a64.is_nan() {
                        i32::MAX as u32 as u64
                    } else {
                        a64.trunc() as i32 as u32 as u64
                    };
                    (r, FpFlags::NONE)
                }
                VectorOp::VFNCvtFXu => {
                    clear_host_fp_flags();
                    let r = std::hint::black_box(vs2_raw as u32 as f32);
                    (r.to_bits() as u64, read_host_fp_flags())
                }
                VectorOp::VFNCvtFX => {
                    clear_host_fp_flags();
                    let r = std::hint::black_box(sign_extend(vs2_raw, src_sew) as i32 as f32);
                    (r.to_bits() as u64, read_host_fp_flags())
                }
                _ => (0, FpFlags::NONE),
            }
        } else {
            (0u64, FpFlags::NONE)
        };

        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
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
    use crate::core::units::vpu::types::{MaskPolicy, TailPolicy, Vlen, Vlmul, Vxrm};

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

    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    #[test]
    fn test_vfadd_f32() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);
        let v2 = VRegIdx::new(2);
        let v3 = VRegIdx::new(3);

        // Write 2.0f32 to v2 elements and 3.0f32 to v1 elements
        for i in 0..4 {
            vpr.write_element(v2, ElemIdx::new(i), Sew::E32, 2.0f32.to_bits() as u64);
            vpr.write_element(v1, ElemIdx::new(i), Sew::E32, 3.0f32.to_bits() as u64);
        }

        let result =
            vec_fp_execute(VectorOp::VFAdd, &mut vpr, v3, v2, VecOperand::Vector(v1), &ctx);

        assert!(!result.vxsat);
        for i in 0..4 {
            let val = vpr.read_element(v3, ElemIdx::new(i), Sew::E32);
            let f = f32::from_bits(val as u32);
            assert_eq!(f, 5.0);
        }
    }

    #[test]
    fn test_vfclass_f32() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);
        let v2 = VRegIdx::new(2);

        // Write: +0.0, -0.0, +inf, qNaN
        vpr.write_element(v1, ElemIdx::new(0), Sew::E32, 0.0f32.to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(1), Sew::E32, (-0.0f32).to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(2), Sew::E32, f32::INFINITY.to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(3), Sew::E32, f32::NAN.to_bits() as u64);

        let _result =
            vec_fp_execute(VectorOp::VFClass, &mut vpr, v2, v1, VecOperand::Scalar(0), &ctx);

        assert_eq!(vpr.read_element(v2, ElemIdx::new(0), Sew::E32), 1 << 4); // +zero
        assert_eq!(vpr.read_element(v2, ElemIdx::new(1), Sew::E32), 1 << 3); // -zero
        assert_eq!(vpr.read_element(v2, ElemIdx::new(2), Sew::E32), 1 << 7); // +inf
        assert_eq!(vpr.read_element(v2, ElemIdx::new(3), Sew::E32), 1 << 9); // qNaN
    }

    #[test]
    fn test_vfmv_sf() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);

        let scalar = 42.0f32.to_bits() as u64;
        let result = vec_fp_execute(
            VectorOp::VFMvSF,
            &mut vpr,
            v1,
            VRegIdx::new(0),
            VecOperand::Scalar(scalar),
            &ctx,
        );

        assert!(result.scalar_result.is_none());
        let val = vpr.read_element(v1, ElemIdx::new(0), Sew::E32);
        assert_eq!(f32::from_bits(val as u32), 42.0);
    }

    #[test]
    fn test_vfmv_fs() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);

        vpr.write_element(v1, ElemIdx::new(0), Sew::E32, 99.5f32.to_bits() as u64);

        let result = vec_fp_execute(
            VectorOp::VFMvFS,
            &mut vpr,
            VRegIdx::new(2),
            v1,
            VecOperand::Scalar(0),
            &ctx,
        );

        let scalar = result.scalar_result.unwrap();
        assert_eq!(f32::from_bits(scalar as u32), 99.5);
    }
}

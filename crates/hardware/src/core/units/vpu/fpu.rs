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

use crate::core::pipeline::signals::VectorOp;
use crate::core::units::fpu::exception_flags::FpFlags;
use crate::core::units::fpu::half::{
    CANONICAL_NAN_F16, classify_f16, f16_to_f32, f64_to_f16, is_snan_f16,
};
use crate::core::units::fpu::nan_handling::{
    box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
};
use crate::core::units::fpu::rounding_modes::RoundingMode;
use crate::core::units::fpu::{
    clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode,
};
use crate::core::units::vpu::alu::{VecExecCtx, VecExecResult, VecOperand};
use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::vpu::types::{ElemIdx, Sew, VRegIdx, Vlmax};

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
pub fn vec_fp_execute(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Set host FPU rounding mode from fcsr.frm for this instruction.
    let saved_rm = set_host_round_mode(ctx.frm);
    let result = vec_fp_dispatch(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    restore_host_round_mode(saved_rm);
    result
}

#[allow(clippy::too_many_lines)]
fn vec_fp_dispatch(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Scalar move: vfmv.f.s — read vs2[0] as scalar result
    // The result is written to a 64-bit FP register; NaN-box sub-64-bit values
    // per RISC-V spec §13.2 (upper bits all-1s for narrower FP widths).
    if op == VectorOp::VFMvFS {
        let raw = vpr.read_element(vs2_idx, ElemIdx::new(0), ctx.sew);
        let val = match ctx.sew {
            Sew::E32 => raw | 0xFFFF_FFFF_0000_0000, // NaN-box f32 in f64 register
            Sew::E16 => raw | 0xFFFF_FFFF_FFFF_0000, // NaN-box f16 in f64 register
            _ => raw,                                // E64: no boxing needed
        };
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
        // Tail elements follow the tail-agnostic rule.
        if ctx.vta.is_agnostic() {
            let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
            let start = if ctx.vl > 0 { 1 } else { 0 };
            for i in start..vlmax {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
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
fn mask_active(vpr: &impl VectorRegFile, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Read operand1 value for element `i` at the given SEW.
#[inline]
fn read_op1(vpr: &impl VectorRegFile, operand1: &VecOperand, i: usize, sew: Sew) -> u64 {
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

// ============================================================================
// vfrsqrt7 / vfrec7 lookup tables (RVV 1.0 §13.9–13.10)
// ============================================================================

/// vfrsqrt7 lookup table. Indexed by `{exp[0], sig[MSB-1:MSB-6]}` (7 bits).
/// `RSQRT7_TABLE[0..64]`  → exp[0]=0,  `RSQRT7_TABLE[64..128]` → exp[0]=1.
#[rustfmt::skip]
static RSQRT7_TABLE: [u8; 128] = [
    // exp[0] = 0
    52, 51, 50, 48, 47, 46, 44, 43, 42, 41, 40, 39, 38, 36, 35, 34,
    33, 32, 31, 30, 30, 29, 28, 27, 26, 25, 24, 24, 23, 22, 21, 21,
    20, 19, 19, 18, 18, 17, 16, 16, 15, 15, 14, 14, 13, 13, 12, 12,
    11, 11, 10, 10,  9,  9,  9,  8,  8,  7,  7,  7,  6,  6,  6,  5,
    // exp[0] = 1
    127, 125, 123, 121, 119, 118, 116, 114, 113, 111, 109, 108, 106, 105, 103, 102,
    100,  99,  97,  96,  95,  93,  92,  91,  90,  88,  87,  86,  85,  84,  83,  82,
     80,  79,  78,  77,  76,  75,  74,  73,  72,  71,  70,  70,  69,  68,  67,  66,
     65,  65,  64,  63,  62,  61,  61,  60,  59,  59,  58,  57,  57,  56,  56,  55,
];

/// vfrec7 lookup table. Indexed by `sig[MSB-1:MSB-7]` (7 bits → 128 entries).
#[rustfmt::skip]
static REC7_TABLE: [u8; 128] = [
    127, 125, 123, 121, 119, 117, 116, 114, 112, 110, 109, 107, 105, 104, 102, 100,
     99,  97,  96,  94,  93,  91,  90,  88,  87,  85,  84,  83,  81,  80,  79,  77,
     76,  75,  74,  72,  71,  70,  69,  68,  66,  65,  64,  63,  62,  61,  60,  59,
     58,  57,  56,  55,  54,  53,  52,  51,  50,  49,  48,  47,  46,  45,  44,  43,
     42,  41,  40,  40,  39,  38,  37,  36,  36,  35,  34,  33,  33,  32,  31,  31,
     30,  29,  29,  28,  27,  27,  26,  26,  25,  24,  24,  23,  23,  22,  22,  21,
     21,  20,  20,  19,  19,  18,  18,  17,  17,  16,  16,  15,  15,  14,  14,  14,
     13,  13,  12,  12,  12,  11,  11,  10,  10,  10,   9,   9,   9,   8,   8,   8,
];

/// Compute vfrsqrt7 for an f32 value. Returns `(result_bits, flags)`.
///
/// Implements the RVV 1.0 §13.9 lookup-table algorithm for a 7-bit accurate
/// approximation of `1/sqrt(x)`.
fn vfrsqrt7_32(bits: u32) -> (u64, FpFlags) {
    let sign = bits >> 31;
    let mut exp = (bits >> 23) as i32 & 0xFF;
    let mut sig = bits & 0x007F_FFFF;

    // Special cases
    if exp == 0xFF && sig != 0 {
        let f = if sig & 0x0040_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, f);
    }
    if exp == 0xFF && sig == 0 {
        if sign != 0 {
            return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, FpFlags::NV);
        }
        return (0xFFFF_FFFF_0000_0000, FpFlags::NONE); // +0.0 NaN-boxed
    }
    if exp == 0 && sig == 0 {
        let r = if sign != 0 { 0xFF80_0000u32 } else { 0x7F80_0000u32 };
        return ((r as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::DZ);
    }
    if sign != 0 {
        return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, FpFlags::NV);
    }

    // Normalize subnormals: shift sig left until bit 22 is set, adjust exp.
    if exp == 0 {
        while sig & 0x0040_0000 == 0 {
            sig <<= 1;
            exp -= 1;
        }
        sig &= !0x0040_0000; // clear the implicit leading 1
    }

    // Lookup: index = {exp[0], sig[21:16]} (7 bits)
    let idx = (((exp & 1) << 6) | ((sig >> 16) as i32 & 0x3F)) as usize;
    let out_sig = RSQRT7_TABLE[idx] as u32;

    // Result exponent: (3 * 127 - 1 - exp) / 2
    let out_exp = ((3 * 127 - 1 - exp) >> 1) as u32;

    let result = (out_exp << 23) | (out_sig << 16);
    ((result as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
}

/// Compute vfrsqrt7 for an f64 value. Returns `(result_bits, flags)`.
fn vfrsqrt7_64(bits: u64) -> (u64, FpFlags) {
    let sign = bits >> 63;
    let mut exp = ((bits >> 52) & 0x7FF) as i32;
    let mut sig = bits & 0x000F_FFFF_FFFF_FFFF;

    if exp == 0x7FF && sig != 0 {
        let f = if sig & 0x0008_0000_0000_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (canonicalize_f64_bits(f64::NAN), f);
    }
    if exp == 0x7FF && sig == 0 {
        if sign != 0 {
            return (canonicalize_f64_bits(f64::NAN), FpFlags::NV);
        }
        return (0, FpFlags::NONE); // +0.0
    }
    if exp == 0 && sig == 0 {
        let r: u64 = if sign != 0 { 0xFFF0_0000_0000_0000 } else { 0x7FF0_0000_0000_0000 };
        return (r, FpFlags::DZ);
    }
    if sign != 0 {
        return (canonicalize_f64_bits(f64::NAN), FpFlags::NV);
    }

    // Normalize subnormals: shift sig left until bit 51 is set.
    if exp == 0 {
        while sig & 0x0008_0000_0000_0000 == 0 {
            sig <<= 1;
            exp -= 1;
        }
        sig &= !0x0008_0000_0000_0000;
    }

    let idx = (((exp & 1) << 6) | ((sig >> 45) as i32 & 0x3F)) as usize;
    let out_sig = RSQRT7_TABLE[idx] as u64;

    let out_exp = ((3 * 1023 - 1 - exp) >> 1) as u64;

    let result = (out_exp << 52) | (out_sig << 45);
    (result, FpFlags::NONE)
}

/// Compute vfrec7 for an f32 value. Returns `(result_bits, flags)`.
///
/// Implements the RVV 1.0 §13.10 lookup-table algorithm for a 7-bit accurate
/// approximation of `1/x`.
fn vfrec7_32(bits: u32) -> (u64, FpFlags) {
    let sign = bits >> 31;
    let mut exp = (bits >> 23) as i32 & 0xFF;
    let mut sig = bits & 0x007F_FFFF;

    // Special cases
    if exp == 0xFF && sig != 0 {
        let f = if sig & 0x0040_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, f);
    }
    if exp == 0xFF && sig == 0 {
        // ±inf → ±0
        let r = sign << 31;
        return ((r as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        let r = (sign << 31) | 0x7F80_0000;
        return ((r as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::DZ);
    }

    // Normalize subnormals
    if exp == 0 {
        while sig & 0x0040_0000 == 0 {
            sig <<= 1;
            exp -= 1;
        }
        sig &= !0x0040_0000;
    }

    // Lookup: index = sig[21:15] (7 bits)
    let idx = ((sig >> 15) & 0x7F) as usize;
    let out_sig = REC7_TABLE[idx] as u32;

    // Result exponent: 2 * 127 - 1 - exp
    let out_exp = 253 - exp;

    if out_exp <= 0 {
        // Subnormal result: add implicit 1, shift right by (1 - out_exp)
        let shift = (1 - out_exp) as u32;
        let result = (sign << 31) | ((out_sig | 0x80) >> shift << 15);
        return ((result as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::NONE);
    }
    if out_exp >= 0xFF {
        let result = (sign << 31) | 0x7F80_0000;
        return ((result as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::OF | FpFlags::NX);
    }

    let result = (sign << 31) | ((out_exp as u32) << 23) | (out_sig << 16);
    ((result as u64) | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
}

/// Compute vfrec7 for an f64 value. Returns `(result_bits, flags)`.
fn vfrec7_64(bits: u64) -> (u64, FpFlags) {
    let sign = bits >> 63;
    let mut exp = ((bits >> 52) & 0x7FF) as i32;
    let mut sig = bits & 0x000F_FFFF_FFFF_FFFF;

    if exp == 0x7FF && sig != 0 {
        let f = if sig & 0x0008_0000_0000_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (canonicalize_f64_bits(f64::NAN), f);
    }
    if exp == 0x7FF && sig == 0 {
        return (sign << 63, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        let r = (sign << 63) | 0x7FF0_0000_0000_0000;
        return (r, FpFlags::DZ);
    }

    // Normalize subnormals
    if exp == 0 {
        while sig & 0x0008_0000_0000_0000 == 0 {
            sig <<= 1;
            exp -= 1;
        }
        sig &= !0x0008_0000_0000_0000;
    }

    let idx = ((sig >> 44) & 0x7F) as usize;
    let out_sig = REC7_TABLE[idx] as u64;

    // Result exponent: 2 * 1023 - 1 - exp
    let out_exp = 2045 - exp;

    if out_exp <= 0 {
        let shift = (1 - out_exp) as u64;
        let result = (sign << 63) | (((out_sig | 0x80) >> shift) << 44);
        return (result, FpFlags::NONE);
    }
    if out_exp >= 0x7FF {
        let result = (sign << 63) | 0x7FF0_0000_0000_0000;
        return (result, FpFlags::OF | FpFlags::NX);
    }

    let result = (sign << 63) | ((out_exp as u64) << 52) | (out_sig << 45);
    (result, FpFlags::NONE)
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
        VectorOp::VFRsqrt7 => vfrsqrt7_32(vs2_bits as u32),
        VectorOp::VFRec7 => vfrec7_32(vs2_bits as u32),
        VectorOp::VFMin => {
            let r = fmin_f32(a, b);
            // IEEE 754-2008 minNum: raise NV if either operand is a signaling NaN.
            let f = if is_snan_f32(a) || is_snan_f32(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, f)
        }
        VectorOp::VFMax => {
            let r = fmax_f32(a, b);
            let f = if is_snan_f32(a) || is_snan_f32(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, f)
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
            clear_host_fp_flags();
            let r = if a.is_nan() {
                u32::MAX as u64
            } else {
                std::hint::black_box(std::hint::black_box(a) as u32) as u64
            };
            // NaN: hardware path skipped, manually report NV.
            let f = read_host_fp_flags() | if a.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
        }
        // Conversions with RTZ: float -> signed int
        VectorOp::VFCvtRtzXF => {
            clear_host_fp_flags();
            let r = if a.is_nan() {
                i32::MAX as u64
            } else {
                std::hint::black_box(std::hint::black_box(a) as i32) as u64
            };
            let f = read_host_fp_flags() | if a.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r & 0xFFFF_FFFF, f)
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
        VectorOp::VFRsqrt7 => vfrsqrt7_64(vs2_bits),
        VectorOp::VFRec7 => vfrec7_64(vs2_bits),
        VectorOp::VFMin => {
            let r = fmin_f64(a, b);
            let f = if is_snan_f64(a) || is_snan_f64(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits(), f)
        }
        VectorOp::VFMax => {
            let r = fmax_f64(a, b);
            let f = if is_snan_f64(a) || is_snan_f64(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits(), f)
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
            clear_host_fp_flags();
            let r = if a.is_nan() {
                u64::MAX
            } else {
                std::hint::black_box(std::hint::black_box(a) as u64)
            };
            let f = read_host_fp_flags() | if a.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
        }
        VectorOp::VFCvtRtzXF => {
            clear_host_fp_flags();
            let r = if a.is_nan() {
                i64::MAX as u64
            } else {
                std::hint::black_box(std::hint::black_box(a) as i64) as u64
            };
            let f = read_host_fp_flags() | if a.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
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
// Per-element FP computation (SEW=16, Zvfh)
// ============================================================================

/// Compute one standard FP element at SEW=16 (Zvfh).
///
/// Vector f16 values are stored as raw u16 bit patterns in the low 16 bits of
/// each element slot — they are NOT NaN-boxed (that is only for scalar registers).
/// Arithmetic is performed by upcasting to f64 (lossless for f16 inputs) and
/// software-rounding back to f16 via `f64_to_f16`.
#[allow(clippy::too_many_lines)]
fn compute_f16(op: VectorOp, vs2_bits: u64, op1_bits: u64, rm: RoundingMode) -> (u64, FpFlags) {
    let ha = vs2_bits as u16;
    let hb = op1_bits as u16;
    let fa = f16_to_f32(ha) as f64;
    let fb = f16_to_f32(hb) as f64;

    // Helper: round f64 result → f16 bit pattern, merging extra flags.
    let round = |val: f64, extra: FpFlags| -> (u64, FpFlags) {
        let (bits, flags) = f64_to_f16(val, rm);
        (bits as u64, flags | extra)
    };

    match op {
        VectorOp::VFAdd => {
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            round(fa + fb, nv)
        }
        VectorOp::VFSub => {
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            round(fa - fb, nv)
        }
        VectorOp::VFRSub => {
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            round(fb - fa, nv)
        }
        VectorOp::VFMul => {
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            round(fa * fb, nv)
        }
        VectorOp::VFDiv => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || is_snan_f16(hb) {
                extra = extra | FpFlags::NV;
            } else if fb == 0.0 && fa != 0.0 && fa.is_finite() {
                extra = extra | FpFlags::DZ;
            } else if fb == 0.0 && fa == 0.0 {
                extra = extra | FpFlags::NV;
            } else if fa.is_infinite() && fb.is_infinite() {
                extra = extra | FpFlags::NV;
            }
            round(fa / fb, extra)
        }
        VectorOp::VFRDiv => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || is_snan_f16(hb) {
                extra = extra | FpFlags::NV;
            } else if fa == 0.0 && fb != 0.0 && fb.is_finite() {
                extra = extra | FpFlags::DZ;
            } else if fa == 0.0 && fb == 0.0 {
                extra = extra | FpFlags::NV;
            } else if fb.is_infinite() && fa.is_infinite() {
                extra = extra | FpFlags::NV;
            }
            round(fb / fa, extra)
        }
        VectorOp::VFSqrt => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || fa < 0.0 {
                extra = extra | FpFlags::NV;
            }
            round(fa.sqrt(), extra)
        }
        VectorOp::VFMin => {
            let f = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            let fa32 = f16_to_f32(ha);
            let fb32 = f16_to_f32(hb);
            let r = fmin_f32(fa32, fb32);
            if r.is_nan() {
                return (CANONICAL_NAN_F16 as u64, f);
            }
            let (bits, _) = f64_to_f16(r as f64, RoundingMode::Rne);
            (bits as u64, f)
        }
        VectorOp::VFMax => {
            let f = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            let fa32 = f16_to_f32(ha);
            let fb32 = f16_to_f32(hb);
            let r = fmax_f32(fa32, fb32);
            if r.is_nan() {
                return (CANONICAL_NAN_F16 as u64, f);
            }
            let (bits, _) = f64_to_f16(r as f64, RoundingMode::Rne);
            (bits as u64, f)
        }
        VectorOp::VFSgnj => ((ha & 0x7FFF) as u64 | (hb & 0x8000) as u64, FpFlags::NONE),
        VectorOp::VFSgnjn => ((ha & 0x7FFF) as u64 | (!hb & 0x8000) as u64, FpFlags::NONE),
        VectorOp::VFSgnjx => ((ha ^ (hb & 0x8000)) as u64, FpFlags::NONE),
        VectorOp::VFClass => (classify_f16(ha), FpFlags::NONE),
        VectorOp::VFCvtXuF => {
            clear_host_fp_flags();
            let r = if fa.is_nan() { u16::MAX as u64 } else { fa as u16 as u64 };
            let f = read_host_fp_flags() | if fa.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
        }
        VectorOp::VFCvtXF => {
            clear_host_fp_flags();
            let r = if fa.is_nan() { i16::MAX as u64 } else { (fa as i16 as u16) as u64 };
            let f = read_host_fp_flags() | if fa.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
        }
        VectorOp::VFCvtRtzXuF => {
            clear_host_fp_flags();
            let r = if fa.is_nan() { u16::MAX as u64 } else { fa as u16 as u64 };
            let f = read_host_fp_flags() | if fa.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
        }
        VectorOp::VFCvtRtzXF => {
            clear_host_fp_flags();
            let r = if fa.is_nan() { i16::MAX as u64 } else { (fa as i16 as u16) as u64 };
            let f = read_host_fp_flags() | if fa.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            (r, f)
        }
        VectorOp::VFCvtFXu => round(vs2_bits as u16 as f64, FpFlags::NONE),
        VectorOp::VFCvtFX => round(sign_extend(vs2_bits, Sew::E16) as i16 as f64, FpFlags::NONE),
        _ => (0, FpFlags::NONE),
    }
}

/// Compute FMA for f16 element (Zvfh).
fn compute_fma_f16(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    vd_bits: u64,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let ha = vs2_bits as u16;
    let hb = op1_bits as u16;
    let hc = vd_bits as u16;
    let vs2 = f16_to_f32(ha) as f64;
    let op1 = f16_to_f32(hb) as f64;
    let vd = f16_to_f32(hc) as f64;

    let nv = if is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc) {
        FpFlags::NV
    } else {
        FpFlags::NONE
    };

    let r = match op {
        VectorOp::VFMacc => op1.mul_add(vs2, vd),
        VectorOp::VFNMacc => (-op1).mul_add(vs2, -vd),
        VectorOp::VFMSac => op1.mul_add(vs2, -vd),
        VectorOp::VFNMSac => (-op1).mul_add(vs2, vd),
        VectorOp::VFMAdd => op1.mul_add(vd, vs2),
        VectorOp::VFNMAdd => (-op1).mul_add(vd, -vs2),
        VectorOp::VFMSub => op1.mul_add(vd, -vs2),
        VectorOp::VFNMSub => (-op1).mul_add(vd, vs2),
        _ => 0.0,
    };

    let (bits, flags) = f64_to_f16(r, rm);
    (bits as u64, flags | nv)
}

// ============================================================================
// Standard element-wise loop
// ============================================================================

/// Standard (non-widening, non-narrowing, non-FMA) FP element-wise loop.
fn exec_fp_standard(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
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

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_f32(op, vs2_val, op1_val),
            Sew::E64 => compute_f64(op, vs2_val, op1_val),
            Sew::E16 if ctx.zvfh => compute_f16(op, vs2_val, op1_val, ctx.frm),
            _ => (0, FpFlags::NONE),
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
    vpr: &mut impl VectorRegFile,
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

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_fma_f32(op, vs2_val, op1_val, vd_val),
            Sew::E64 => compute_fma_f64(op, vs2_val, op1_val, vd_val),
            Sew::E16 if ctx.zvfh => compute_fma_f16(op, vs2_val, op1_val, vd_val, ctx.frm),
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
///
/// Mask-producing instructions write one bit per element. The tail comprises
/// bits `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3), so
/// the loop must iterate over all VLEN mask bits, not just VLMAX elements.
fn exec_fp_comparison(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Mask registers hold VLEN bits; the tail extends from vl to VLEN-1.
    let vlen_bits = vpr.vlen().bits();
    let mut flags = FpFlags::NONE;

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
            Sew::E16 if ctx.zvfh => {
                let a16 = vs2_val as u16;
                let b16 = op1_val as u16;
                let a = f16_to_f32(a16);
                let b = f16_to_f32(b16);
                let nv = match op {
                    VectorOp::VMFEq | VectorOp::VMFNe => is_snan_f16(a16) || is_snan_f16(b16),
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
    vpr: &mut impl VectorRegFile,
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
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };

    // Handle widening FMA BEFORE the main loop — the FMA reads vd as the
    // accumulator, so we must not overwrite it with the arithmetic path first.
    if matches!(op, VectorOp::VFWMacc | VectorOp::VFWNMacc | VectorOp::VFWMSac | VectorOp::VFWNMSac)
    {
        return exec_fp_widening_fma(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    // Determine if this is a ".w" variant (vs2 is already wide)
    let vs2_wide = matches!(op, VectorOp::VFWAddW | VectorOp::VFWSubW);

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
                        if a.is_nan() {
                            u64::MAX
                        } else {
                            std::hint::black_box(std::hint::black_box(a) as u64)
                        }
                    }
                    VectorOp::VFWCvtRtzXF => {
                        let a = elem_to_f32(vs2_raw);
                        if a.is_nan() {
                            i64::MAX as u64
                        } else {
                            std::hint::black_box(std::hint::black_box(a) as i64) as u64
                        }
                    }
                    VectorOp::VFWCvtFXu => (vs2_raw as u32 as f64).to_bits(),
                    VectorOp::VFWCvtFX => (sign_extend(vs2_raw, ctx.sew) as i32 as f64).to_bits(),
                    _ => 0,
                };
                // RTZ variants previously skipped flag reporting. All float→int
                // conversions must report NV for NaN/out-of-range and NX for inexact.
                let nan_input = matches!(
                    op,
                    VectorOp::VFWCvtRtzXuF
                        | VectorOp::VFWCvtRtzXF
                        | VectorOp::VFWCvtXuF
                        | VectorOp::VFWCvtXF
                ) && elem_to_f32(vs2_raw).is_nan();
                let f = read_host_fp_flags() | if nan_input { FpFlags::NV } else { FpFlags::NONE };
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
        } else if ctx.sew == Sew::E16 && ctx.zvfh {
            // Zvfh widening: SEW=16 (f16) -> wsew=E32 (f32)
            // Handle integer-producing conversions
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
                let a16 = vs2_raw as u16;
                let a_f = f16_to_f32(a16);
                let bits = match op {
                    VectorOp::VFWCvtXuF | VectorOp::VFWCvtRtzXuF => {
                        if a_f.is_nan() { u32::MAX as u64 } else { a_f as u32 as u64 }
                    }
                    VectorOp::VFWCvtXF | VectorOp::VFWCvtRtzXF => {
                        if a_f.is_nan() { i32::MAX as u64 } else { a_f as i32 as u32 as u64 }
                    }
                    VectorOp::VFWCvtFXu => (vs2_raw as u16 as f32).to_bits() as u64,
                    VectorOp::VFWCvtFX => {
                        (sign_extend(vs2_raw, Sew::E16) as i16 as f32).to_bits() as u64
                    }
                    _ => 0,
                };
                let nan_input = matches!(
                    op,
                    VectorOp::VFWCvtXuF
                        | VectorOp::VFWCvtXF
                        | VectorOp::VFWCvtRtzXuF
                        | VectorOp::VFWCvtRtzXF
                ) && a_f.is_nan();
                let f =
                    read_host_fp_flags() | if nan_input { FpFlags::NV } else { FpFlags::NONE };
                flags = flags | f;
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, bits);
                continue;
            }

            // FP arithmetic widening: f16 inputs → f32 output (via f64 for lossless computation)
            let vs2_f = if vs2_wide {
                elem_to_f32(vs2_raw) as f64
            } else {
                f16_to_f32(vs2_raw as u16) as f64
            };
            let op1_f = f16_to_f32(op1_raw as u16) as f64;

            clear_host_fp_flags();
            let r_f64 = std::hint::black_box(match op {
                VectorOp::VFWAdd | VectorOp::VFWAddW => {
                    std::hint::black_box(vs2_f) + std::hint::black_box(op1_f)
                }
                VectorOp::VFWSub | VectorOp::VFWSubW => {
                    std::hint::black_box(vs2_f) - std::hint::black_box(op1_f)
                }
                VectorOp::VFWMul => std::hint::black_box(vs2_f) * std::hint::black_box(op1_f),
                VectorOp::VFWCvtFF => vs2_f, // f16→f32 conversion (just widen, exact)
                _ => vs2_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            vpr.write_element(
                vd_idx,
                ElemIdx::new(i),
                wsew,
                box_f32_canon(r_f64 as f32),
            );
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Widening FMA operations.
fn exec_fp_widening_fma(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
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
        } else if ctx.sew == Sew::E16 && ctx.zvfh {
            // Zvfh widening FMA: f16 inputs → f32 accumulator
            let vs2_f = f16_to_f32(vs2_raw as u16) as f64;
            let op1_f = f16_to_f32(op1_raw as u16) as f64;
            let vd_f = elem_to_f32(vd_raw) as f64;

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
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, box_f32_canon(r as f32));
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
    vpr: &mut impl VectorRegFile,
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

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);

        // Narrowing: src_sew=E64 -> dst_sew=E32
        let (result, f) = if ctx.sew == Sew::E32 {
            let a64 = elem_to_f64(vs2_raw);
            match op {
                VectorOp::VFNCvtFF => {
                    clear_host_fp_flags();
                    let r = std::hint::black_box(std::hint::black_box(a64) as f32);
                    (box_f32_canon(r) & 0xFFFF_FFFF, read_host_fp_flags())
                }
                VectorOp::VFNCvtRodFF => {
                    // Round-to-odd (jamming): if the f64→f32 conversion is
                    // inexact, set the LSB of the f32 mantissa to 1. This
                    // prevents double-rounding errors in chained narrowing
                    // (e.g. f64→f32→f16). Per SoftFloat semantics, round-to-odd
                    // does not raise the inexact (NX) flag.
                    clear_host_fp_flags();
                    let r = std::hint::black_box(std::hint::black_box(a64) as f32);
                    let hw_flags = read_host_fp_flags();
                    if !r.is_nan() && !r.is_infinite() && (r as f64) != a64 {
                        // Inexact: jam LSB to 1, suppress NX
                        let jammed = f32::from_bits(r.to_bits() | 1);
                        let flags_no_nx = FpFlags::from_bits(hw_flags.bits() & !FpFlags::NX.bits());
                        (box_f32_canon(jammed) & 0xFFFF_FFFF, flags_no_nx)
                    } else {
                        (box_f32_canon(r) & 0xFFFF_FFFF, hw_flags)
                    }
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
                    clear_host_fp_flags();
                    let r = if a64.is_nan() {
                        u32::MAX as u64
                    } else {
                        std::hint::black_box(std::hint::black_box(a64) as u32) as u64
                    };
                    let f = read_host_fp_flags()
                        | if a64.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtRtzXF => {
                    clear_host_fp_flags();
                    let r = if a64.is_nan() {
                        i32::MAX as u32 as u64
                    } else {
                        std::hint::black_box(std::hint::black_box(a64) as i32) as u32 as u64
                    };
                    let f = read_host_fp_flags()
                        | if a64.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtFXu => {
                    // Convert full 2*SEW unsigned integer to SEW float.
                    // Must not truncate to u32 first — the source is a 64-bit integer.
                    clear_host_fp_flags();
                    let r = std::hint::black_box(vs2_raw as f32);
                    (r.to_bits() as u64, read_host_fp_flags())
                }
                VectorOp::VFNCvtFX => {
                    // Convert full 2*SEW signed integer to SEW float.
                    // Must not truncate to i32 first — the source is a 64-bit integer.
                    clear_host_fp_flags();
                    let r = std::hint::black_box(sign_extend(vs2_raw, src_sew) as f32);
                    (r.to_bits() as u64, read_host_fp_flags())
                }
                _ => (0, FpFlags::NONE),
            }
        } else if ctx.sew == Sew::E16 && ctx.zvfh {
            // Zvfh narrowing: src_sew=E32 (f32) -> dst_sew=E16 (f16)
            let a32 = elem_to_f32(vs2_raw);
            match op {
                VectorOp::VFNCvtFF => {
                    // f32 -> f16 with rounding mode
                    let (bits, f) = f64_to_f16(a32 as f64, ctx.frm);
                    (bits as u64, f)
                }
                VectorOp::VFNCvtRodFF => {
                    // Round-to-odd: suppress double-rounding for chained narrowing
                    let r32 = a32;
                    if !r32.is_nan() && !r32.is_infinite() {
                        let (bits, f) = f64_to_f16(r32 as f64, ctx.frm);
                        let flags_no_nx = FpFlags::from_bits(f.bits() & !FpFlags::NX.bits());
                        // Jam LSB if inexact
                        let jammed = if (r32 as f64) != (r32 as f64)
                            && (r32 as f64).is_finite()
                        {
                            bits | 1
                        } else {
                            bits
                        };
                        (jammed as u64, flags_no_nx)
                    } else {
                        let (bits, f) = f64_to_f16(r32 as f64, ctx.frm);
                        (bits as u64, f)
                    }
                }
                VectorOp::VFNCvtXuF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() { u16::MAX as u64 } else { a32 as u16 as u64 };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtXF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() {
                        i16::MAX as u64
                    } else {
                        a32 as i16 as u16 as u64
                    };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtRtzXuF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() {
                        u16::MAX as u64
                    } else {
                        std::hint::black_box(a32) as u16 as u64
                    };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtRtzXF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() {
                        i16::MAX as u64
                    } else {
                        std::hint::black_box(a32) as i16 as u16 as u64
                    };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtFXu => {
                    // 2*SEW unsigned int (u32) → f16
                    let (bits, f) = f64_to_f16(vs2_raw as u32 as f64, ctx.frm);
                    (bits as u64, f)
                }
                VectorOp::VFNCvtFX => {
                    // 2*SEW signed int (i32) → f16
                    let (bits, f) =
                        f64_to_f16(sign_extend(vs2_raw, src_sew) as i32 as f64, ctx.frm);
                    (bits as u64, f)
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
    use crate::core::arch::vpr::Vpr;
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
            frm: RoundingMode::Rne,
            zvfh: false,
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

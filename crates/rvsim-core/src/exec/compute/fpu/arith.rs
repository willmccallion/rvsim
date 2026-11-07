//! Single- and double-precision operations and classification.

use crate::isa::fp::{FpClass, FpFormat};
use crate::isa::op::AluOp;

use super::nan_handling::{
    box_f32, box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
    unbox_f32,
};

/// Bit mask for the sign bit in a 32-bit IEEE 754 float (bit 31).
pub(super) const F32_SIGN_BIT: u32 = 0x8000_0000;

/// Bit mask for the sign bit in a 64-bit IEEE 754 float (bit 63).
pub(super) const F64_SIGN_BIT: u64 = 0x8000_0000_0000_0000;

// Integer range boundaries as f64 for float-to-integer conversion range checks.
// Values at or beyond these limits overflow the target integer type.

/// Single-precision (f32) execution path.
///
/// Inputs are unboxed with NaN-boxing validation. Arithmetic results
/// are canonicalized and re-boxed before returning.
pub(super) fn execute_f32(op: AluOp, a: u64, b: u64, c: u64) -> u64 {
    // Unbox with NaN-boxing validation (RISC-V spec §12.2).
    let fa = unbox_f32(a);
    let fb = unbox_f32(b);
    let fc = unbox_f32(c);

    match op {
        // Results canonicalize a NaN.
        AluOp::FAdd => box_f32_canon(fa + fb),
        AluOp::FSub => box_f32_canon(fa - fb),
        AluOp::FMul => box_f32_canon(fa * fb),
        AluOp::FDiv => box_f32_canon(fa / fb),
        AluOp::FSqrt => box_f32_canon(fa.sqrt()),

        // IEEE 754-2008 minNum/maxNum.
        AluOp::FMin => box_f32(fmin_f32(fa, fb)),
        AluOp::FMax => box_f32(fmax_f32(fa, fb)),

        // Results canonicalize a NaN.
        AluOp::FMAdd => box_f32_canon(fa.mul_add(fb, fc)),
        AluOp::FMSub => box_f32_canon(fa.mul_add(fb, -fc)),
        AluOp::FNMAdd => box_f32_canon((-fa).mul_add(fb, -fc)),
        AluOp::FNMSub => box_f32_canon((-fa).mul_add(fb, fc)),

        // On the raw bits: no canonicalization.
        AluOp::FSgnJ => {
            box_f32(f32::from_bits((fa.to_bits() & !F32_SIGN_BIT) | (fb.to_bits() & F32_SIGN_BIT)))
        }
        AluOp::FSgnJN => {
            box_f32(f32::from_bits((fa.to_bits() & !F32_SIGN_BIT) | (!fb.to_bits() & F32_SIGN_BIT)))
        }
        AluOp::FSgnJX => box_f32(f32::from_bits(fa.to_bits() ^ (fb.to_bits() & F32_SIGN_BIT))),

        // An integer 0 or 1, not NaN-boxed.
        AluOp::FEq => (fa == fb) as u64,
        AluOp::FLt => (fa < fb) as u64,
        AluOp::FLe => (fa <= fb) as u64,
        AluOp::FClass => FpClass::of(FpFormat::Single, u64::from(fa.to_bits())).mask(),
        AluOp::FMvToX => (a as i32) as u64,

        _ => 0,
    }
}

/// Double-precision (f64) execution path.
///
/// Arithmetic results are canonicalized before returning.
pub(super) fn execute_f64(op: AluOp, a: u64, b: u64, c: u64) -> u64 {
    let fa = f64::from_bits(a);
    let fb = f64::from_bits(b);
    let fc = f64::from_bits(c);

    match op {
        // Results canonicalize a NaN.
        AluOp::FAdd => canonicalize_f64_bits(fa + fb),
        AluOp::FSub => canonicalize_f64_bits(fa - fb),
        AluOp::FMul => canonicalize_f64_bits(fa * fb),
        AluOp::FDiv => canonicalize_f64_bits(fa / fb),
        AluOp::FSqrt => canonicalize_f64_bits(fa.sqrt()),

        // IEEE 754-2008 minNum/maxNum.
        AluOp::FMin => fmin_f64(fa, fb).to_bits(),
        AluOp::FMax => fmax_f64(fa, fb).to_bits(),

        // Results canonicalize a NaN.
        AluOp::FMAdd => canonicalize_f64_bits(fa.mul_add(fb, fc)),
        AluOp::FMSub => canonicalize_f64_bits(fa.mul_add(fb, -fc)),
        AluOp::FNMAdd => canonicalize_f64_bits((-fa).mul_add(fb, -fc)),
        AluOp::FNMSub => canonicalize_f64_bits((-fa).mul_add(fb, fc)),
        AluOp::FSgnJ => {
            f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (fb.to_bits() & F64_SIGN_BIT)).to_bits()
        }
        AluOp::FSgnJN => {
            f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (!fb.to_bits() & F64_SIGN_BIT))
                .to_bits()
        }
        AluOp::FSgnJX => f64::from_bits(fa.to_bits() ^ (fb.to_bits() & F64_SIGN_BIT)).to_bits(),
        AluOp::FEq => (fa == fb) as u64,
        AluOp::FLt => (fa < fb) as u64,
        AluOp::FLe => (fa <= fb) as u64,
        AluOp::FClass => FpClass::of(FpFormat::Double, fa.to_bits()).mask(),

        // No NaN-boxing in the 64-bit path.
        AluOp::FMvToX => a,

        _ => 0,
    }
}

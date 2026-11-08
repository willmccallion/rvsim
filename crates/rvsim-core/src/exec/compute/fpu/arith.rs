//! Single- and double-precision operations and classification.

use crate::isa::fp::{FpClass, FpFormat};
use crate::isa::op::{FmaSigns, FpArithOp, FpCompareOp, FpOp, SignInjectOp};

use super::nan_handling::{
    box_f32, box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
    unbox_f32,
};

/// Bit mask for the sign bit in a 32-bit IEEE 754 float (bit 31).
pub(super) const F32_SIGN_BIT: u32 = 0x8000_0000;

/// Bit mask for the sign bit in a 64-bit IEEE 754 float (bit 63).
pub(super) const F64_SIGN_BIT: u64 = 0x8000_0000_0000_0000;

/// Single-precision (f32) execution path.
///
/// Inputs are unboxed with NaN-boxing validation. Arithmetic results
/// are canonicalized and re-boxed before returning.
pub(super) fn execute_f32(op: FpOp, a: u64, b: u64, c: u64) -> u64 {
    // Unbox with NaN-boxing validation (RISC-V spec §12.2).
    let fa = unbox_f32(a);
    let fb = unbox_f32(b);
    let fc = unbox_f32(c);

    match op {
        // Results canonicalize a NaN.
        FpOp::Arith(FpArithOp::Add) => box_f32_canon(fa + fb),
        FpOp::Arith(FpArithOp::Sub) => box_f32_canon(fa - fb),
        FpOp::Arith(FpArithOp::Mul) => box_f32_canon(fa * fb),
        FpOp::Arith(FpArithOp::Div) => box_f32_canon(fa / fb),
        FpOp::Sqrt => box_f32_canon(fa.sqrt()),

        // IEEE 754-2008 minNum/maxNum.
        FpOp::Min => box_f32(fmin_f32(fa, fb)),
        FpOp::Max => box_f32(fmax_f32(fa, fb)),

        // Results canonicalize a NaN.
        FpOp::Fma(FmaSigns::Macc) => box_f32_canon(fa.mul_add(fb, fc)),
        FpOp::Fma(FmaSigns::MSac) => box_f32_canon(fa.mul_add(fb, -fc)),
        FpOp::Fma(FmaSigns::NMacc) => box_f32_canon((-fa).mul_add(fb, -fc)),
        FpOp::Fma(FmaSigns::NMSac) => box_f32_canon((-fa).mul_add(fb, fc)),

        // On the raw bits: no canonicalization.
        FpOp::SignInject(SignInjectOp::Copy) => {
            box_f32(f32::from_bits((fa.to_bits() & !F32_SIGN_BIT) | (fb.to_bits() & F32_SIGN_BIT)))
        }
        FpOp::SignInject(SignInjectOp::Negate) => {
            box_f32(f32::from_bits((fa.to_bits() & !F32_SIGN_BIT) | (!fb.to_bits() & F32_SIGN_BIT)))
        }
        FpOp::SignInject(SignInjectOp::Xor) => {
            box_f32(f32::from_bits(fa.to_bits() ^ (fb.to_bits() & F32_SIGN_BIT)))
        }

        // An integer 0 or 1, not NaN-boxed.
        FpOp::Compare(FpCompareOp::Eq) => (fa == fb) as u64,
        FpOp::Compare(FpCompareOp::Lt) => (fa < fb) as u64,
        FpOp::Compare(FpCompareOp::Le) => (fa <= fb) as u64,
        FpOp::Class => FpClass::of(FpFormat::Single, u64::from(fa.to_bits())).mask(),
        FpOp::MoveToInt => (a as i32) as u64,

        _ => 0,
    }
}

/// Double-precision (f64) execution path.
///
/// Arithmetic results are canonicalized before returning.
pub(super) fn execute_f64(op: FpOp, a: u64, b: u64, c: u64) -> u64 {
    let fa = f64::from_bits(a);
    let fb = f64::from_bits(b);
    let fc = f64::from_bits(c);

    match op {
        // Results canonicalize a NaN.
        FpOp::Arith(FpArithOp::Add) => canonicalize_f64_bits(fa + fb),
        FpOp::Arith(FpArithOp::Sub) => canonicalize_f64_bits(fa - fb),
        FpOp::Arith(FpArithOp::Mul) => canonicalize_f64_bits(fa * fb),
        FpOp::Arith(FpArithOp::Div) => canonicalize_f64_bits(fa / fb),
        FpOp::Sqrt => canonicalize_f64_bits(fa.sqrt()),

        // IEEE 754-2008 minNum/maxNum.
        FpOp::Min => fmin_f64(fa, fb).to_bits(),
        FpOp::Max => fmax_f64(fa, fb).to_bits(),

        // Results canonicalize a NaN.
        FpOp::Fma(FmaSigns::Macc) => canonicalize_f64_bits(fa.mul_add(fb, fc)),
        FpOp::Fma(FmaSigns::MSac) => canonicalize_f64_bits(fa.mul_add(fb, -fc)),
        FpOp::Fma(FmaSigns::NMacc) => canonicalize_f64_bits((-fa).mul_add(fb, -fc)),
        FpOp::Fma(FmaSigns::NMSac) => canonicalize_f64_bits((-fa).mul_add(fb, fc)),
        FpOp::SignInject(SignInjectOp::Copy) => {
            f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (fb.to_bits() & F64_SIGN_BIT)).to_bits()
        }
        FpOp::SignInject(SignInjectOp::Negate) => {
            f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (!fb.to_bits() & F64_SIGN_BIT))
                .to_bits()
        }
        FpOp::SignInject(SignInjectOp::Xor) => {
            f64::from_bits(fa.to_bits() ^ (fb.to_bits() & F64_SIGN_BIT)).to_bits()
        }
        FpOp::Compare(FpCompareOp::Eq) => (fa == fb) as u64,
        FpOp::Compare(FpCompareOp::Lt) => (fa < fb) as u64,
        FpOp::Compare(FpCompareOp::Le) => (fa <= fb) as u64,
        FpOp::Class => FpClass::of(FpFormat::Double, fa.to_bits()).mask(),

        // No NaN-boxing in the 64-bit path.
        FpOp::MoveToInt => a,

        _ => 0,
    }
}

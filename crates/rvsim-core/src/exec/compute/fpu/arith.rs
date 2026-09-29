//! Single- and double-precision operations and classification.

use crate::isa::op::AluOp;

use super::convert::{f64_to_i32_rv, f64_to_i64_rv, f64_to_u32_rv, f64_to_u64_rv};
use super::nan_handling::{
    box_f32, box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
    unbox_f32,
};

/// RISC-V FCLASS result for f32: classify into one of 10 categories.
pub(super) const fn classify_f32(sign: u32, exp: u32, frac: u32) -> u32 {
    if exp == 0xFF && frac != 0 {
        // NaN
        if frac & 0x0040_0000 != 0 {
            1 << 9 // qNaN
        } else {
            1 << 8 // sNaN
        }
    } else if exp == 0xFF && frac == 0 {
        if sign != 0 { 1 << 0 } else { 1 << 7 } // ±inf
    } else if exp == 0 && frac == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 } // ±zero
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 } // ±subnormal
    } else if sign != 0 {
        1 << 1
    } else {
        1 << 6
    } // ±normal
}

/// RISC-V FCLASS result for f64: classify into one of 10 categories.
pub(super) const fn classify_f64(sign: u64, exp: u64, frac: u64) -> u64 {
    if exp == 0x7FF && frac != 0 {
        if frac & 0x0008_0000_0000_0000 != 0 {
            1 << 9 // qNaN
        } else {
            1 << 8 // sNaN
        }
    } else if exp == 0x7FF && frac == 0 {
        if sign != 0 { 1 << 0 } else { 1 << 7 } // ±inf
    } else if exp == 0 && frac == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 } // ±zero
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 } // ±subnormal
    } else if sign != 0 {
        1 << 1
    } else {
        1 << 6
    } // ±normal
}

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
        // --- Arithmetic (canonicalize NaN results) ---
        AluOp::FAdd => box_f32_canon(fa + fb),
        AluOp::FSub => box_f32_canon(fa - fb),
        AluOp::FMul => box_f32_canon(fa * fb),
        AluOp::FDiv => box_f32_canon(fa / fb),
        AluOp::FSqrt => box_f32_canon(fa.sqrt()),

        // --- Min/Max (IEEE 754-2008 minNum/maxNum) ---
        AluOp::FMin => box_f32(fmin_f32(fa, fb)),
        AluOp::FMax => box_f32(fmax_f32(fa, fb)),

        // --- Fused multiply-add family (canonicalize) ---
        AluOp::FMAdd => box_f32_canon(fa.mul_add(fb, fc)),
        AluOp::FMSub => box_f32_canon(fa.mul_add(fb, -fc)),
        AluOp::FNMAdd => box_f32_canon((-fa).mul_add(fb, -fc)),
        AluOp::FNMSub => box_f32_canon((-fa).mul_add(fb, fc)),

        // --- Sign injection (operates on raw bits, no canonicalization) ---
        AluOp::FSgnJ => {
            box_f32(f32::from_bits((fa.to_bits() & !F32_SIGN_BIT) | (fb.to_bits() & F32_SIGN_BIT)))
        }
        AluOp::FSgnJN => {
            box_f32(f32::from_bits((fa.to_bits() & !F32_SIGN_BIT) | (!fb.to_bits() & F32_SIGN_BIT)))
        }
        AluOp::FSgnJX => box_f32(f32::from_bits(fa.to_bits() ^ (fb.to_bits() & F32_SIGN_BIT))),

        // --- Comparisons (return integer 0 or 1, not boxed) ---
        AluOp::FEq => (fa == fb) as u64,
        AluOp::FLt => (fa < fb) as u64,
        AluOp::FLe => (fa <= fb) as u64,

        // --- Classify ---
        AluOp::FClass => {
            let bits = fa.to_bits();
            let sign = (bits >> 31) & 1;
            let exp = (bits >> 23) & 0xFF;
            let frac = bits & 0x007F_FFFF;
            classify_f32(sign, exp, frac) as u64
        }

        // --- Conversions (float → integer) ---
        // RV64: W-sized results are sign-extended to 64 bits (even unsigned).
        // NaN → positive max per RISC-V spec.
        AluOp::FCvtWS => f64_to_i32_rv(fa as f64) as i64 as u64,
        AluOp::FCvtWUS => f64_to_u32_rv(fa as f64) as i32 as i64 as u64,
        AluOp::FCvtLS => f64_to_i64_rv(fa as f64) as u64,
        AluOp::FCvtLUS => f64_to_u64_rv(fa as f64),

        // --- Conversions (double → single, identity in f32 path) ---
        AluOp::FCvtSD => box_f32_canon(fa),

        // --- Conversions (integer → float, use raw `a` for integer bits) ---
        AluOp::FCvtSW => ((a as i32) as f64).to_bits(),
        AluOp::FCvtSWU => ((a as u32) as f64).to_bits(),
        AluOp::FCvtSL => ((a as i64) as f64).to_bits(),
        AluOp::FCvtSLU => (a as f64).to_bits(),

        // --- Conversions (single → double) ---
        AluOp::FCvtDS => (unbox_f32(a) as f64).to_bits(),

        // --- Move operations ---
        AluOp::FMvToF => box_f32(f32::from_bits(a as u32)),
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
        // --- Arithmetic (canonicalize NaN results) ---
        AluOp::FAdd => canonicalize_f64_bits(fa + fb),
        AluOp::FSub => canonicalize_f64_bits(fa - fb),
        AluOp::FMul => canonicalize_f64_bits(fa * fb),
        AluOp::FDiv => canonicalize_f64_bits(fa / fb),
        AluOp::FSqrt => canonicalize_f64_bits(fa.sqrt()),

        // --- Min/Max (IEEE 754-2008 minNum/maxNum) ---
        AluOp::FMin => fmin_f64(fa, fb).to_bits(),
        AluOp::FMax => fmax_f64(fa, fb).to_bits(),

        // --- Fused multiply-add family (canonicalize) ---
        AluOp::FMAdd => canonicalize_f64_bits(fa.mul_add(fb, fc)),
        AluOp::FMSub => canonicalize_f64_bits(fa.mul_add(fb, -fc)),
        AluOp::FNMAdd => canonicalize_f64_bits((-fa).mul_add(fb, -fc)),
        AluOp::FNMSub => canonicalize_f64_bits((-fa).mul_add(fb, fc)),

        // --- Sign injection ---
        AluOp::FSgnJ => {
            f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (fb.to_bits() & F64_SIGN_BIT)).to_bits()
        }
        AluOp::FSgnJN => {
            f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (!fb.to_bits() & F64_SIGN_BIT))
                .to_bits()
        }
        AluOp::FSgnJX => f64::from_bits(fa.to_bits() ^ (fb.to_bits() & F64_SIGN_BIT)).to_bits(),

        // --- Comparisons ---
        AluOp::FEq => (fa == fb) as u64,
        AluOp::FLt => (fa < fb) as u64,
        AluOp::FLe => (fa <= fb) as u64,

        // --- Classify ---
        AluOp::FClass => {
            let bits = fa.to_bits();
            let sign = (bits >> 63) & 1;
            let exp = (bits >> 52) & 0x7FF;
            let frac = bits & 0x000F_FFFF_FFFF_FFFF;
            classify_f64(sign, exp, frac)
        }

        // --- Conversions ---
        // RV64: W-sized results are sign-extended to 64 bits (even unsigned).
        // NaN → positive max per RISC-V spec.
        AluOp::FCvtWS => f64_to_i32_rv(fa) as i64 as u64,
        AluOp::FCvtWUS => f64_to_u32_rv(fa) as i32 as i64 as u64,
        AluOp::FCvtLS => f64_to_i64_rv(fa) as u64,
        AluOp::FCvtLUS => f64_to_u64_rv(fa),
        AluOp::FCvtSD => box_f32_canon(fa as f32),
        AluOp::FCvtSW => ((a as i32) as f64).to_bits(),
        AluOp::FCvtSWU => ((a as u32) as f64).to_bits(),
        AluOp::FCvtSL => ((a as i64) as f64).to_bits(),
        AluOp::FCvtSLU => (a as f64).to_bits(),

        // --- Move operations (64-bit path: no boxing needed) ---
        AluOp::FMvToF | AluOp::FMvToX => a,

        _ => 0,
    }
}

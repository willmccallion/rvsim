//! NaN boxing, unboxing, and canonical NaN propagation for the FPU.
//!
//! RISC-V stores single-precision (f32) values in 64-bit floating-point
//! registers using "NaN boxing": the upper 32 bits must be all 1s.
//!
//! - **Boxing** ([`box_f32`]): Sets upper 32 bits to 1s when writing an f32
//!   result into a 64-bit register.
//! - **Unboxing** ([`unbox_f32`]): Checks that the upper 32 bits are all 1s.
//!   If not, the value is treated as canonical NaN (RISC-V spec §12.2).
//! - **Canonicalization** ([`canonicalize_f32`], [`canonicalize_f64`]): Any NaN
//!   result from an arithmetic operation is replaced with the canonical quiet
//!   NaN, discarding payload bits (RISC-V spec §11.3).

/// Canonical quiet NaN for IEEE 754 single-precision (positive, quiet, zero payload).
const CANONICAL_NAN_F32: u32 = 0x7fc0_0000;

/// Canonical quiet NaN for IEEE 754 double-precision (positive, quiet, zero payload).
const CANONICAL_NAN_F64: u64 = 0x7ff8_0000_0000_0000;

/// Upper-32-bit mask used for NaN boxing validation.
const NAN_BOX_MASK: u64 = 0xFFFF_FFFF_0000_0000;

/// Boxes an f32 value into a 64-bit NaN-boxed representation.
///
/// Sets the upper 32 bits to all 1s, per RISC-V spec §12.2.
///
/// # Arguments
///
/// * `f` - The 32-bit floating-point value to box.
///
/// # Returns
///
/// A 64-bit value with the f32 in the lower 32 bits and all 1s in the upper 32 bits.
#[inline]
pub fn box_f32(f: f32) -> u64 {
    (f.to_bits() as u64) | NAN_BOX_MASK
}

/// Unboxes a 64-bit register value to obtain an f32.
///
/// Validates that the upper 32 bits are all 1s. If valid, returns the
/// lower 32 bits interpreted as f32. If invalid (not properly NaN-boxed),
/// returns the canonical NaN per RISC-V spec §12.2.
///
/// # Arguments
///
/// * `val` - The 64-bit register value to unbox.
///
/// # Returns
///
/// The unboxed f32, or canonical NaN if the value was not properly NaN-boxed.
#[inline]
pub fn unbox_f32(val: u64) -> f32 {
    if (val & NAN_BOX_MASK) == NAN_BOX_MASK {
        f32::from_bits(val as u32)
    } else {
        // Not properly NaN-boxed: treat as canonical NaN (RISC-V spec §12.2).
        f32::from_bits(CANONICAL_NAN_F32)
    }
}

/// Canonicalizes a single-precision floating-point result.
///
/// If the value is any kind of NaN (quiet or signaling, any payload),
/// it is replaced with the canonical quiet NaN (`0x7fc00000`).
/// Non-NaN values pass through unchanged.
///
/// This ensures RISC-V compliance: all NaN results must be the canonical
/// quiet NaN (RISC-V spec §11.3).
///
/// # Arguments
///
/// * `f` - The f32 result to canonicalize.
///
/// # Returns
///
/// The original value if not NaN, or canonical NaN if it was any NaN.
#[inline]
pub fn canonicalize_f32(f: f32) -> f32 {
    if f.is_nan() {
        f32::from_bits(CANONICAL_NAN_F32)
    } else {
        f
    }
}

/// Canonicalizes a double-precision floating-point result.
///
/// If the value is any kind of NaN (quiet or signaling, any payload),
/// it is replaced with the canonical quiet NaN (`0x7ff8000000000000`).
/// Non-NaN values pass through unchanged.
///
/// # Arguments
///
/// * `f` - The f64 result to canonicalize.
///
/// # Returns
///
/// The original value if not NaN, or canonical NaN if it was any NaN.
#[inline]
pub fn canonicalize_f64(f: f64) -> f64 {
    if f.is_nan() {
        f64::from_bits(CANONICAL_NAN_F64)
    } else {
        f
    }
}

/// IEEE 754-2008 `minNum` for single-precision (RISC-V FMIN.S).
///
/// If exactly one operand is NaN, returns the non-NaN operand.
/// If both are NaN, returns canonical NaN.
/// Otherwise returns the arithmetic minimum.
///
/// Note: sNaN inputs set the Invalid flag (handled at a higher level
/// once exception flags are implemented).
///
/// # Arguments
///
/// * `a` - First f32 operand.
/// * `b` - Second f32 operand.
///
/// # Returns
///
/// The IEEE 754-2008 minimum, with canonical NaN propagation.
#[inline]
pub fn fmin_f32(a: f32, b: f32) -> f32 {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => f32::from_bits(CANONICAL_NAN_F32),
        (true, false) => b,
        (false, true) => a,
        (false, false) => {
            // IEEE 754-2008: -0.0 < +0.0
            if a.to_bits() == 0x8000_0000 && b.to_bits() == 0x0000_0000 {
                a // -0.0
            } else if b.to_bits() == 0x8000_0000 && a.to_bits() == 0x0000_0000 {
                b // -0.0
            } else {
                a.min(b)
            }
        }
    }
}

/// IEEE 754-2008 `maxNum` for single-precision (RISC-V FMAX.S).
///
/// If exactly one operand is NaN, returns the non-NaN operand.
/// If both are NaN, returns canonical NaN.
/// Otherwise returns the arithmetic maximum.
///
/// # Arguments
///
/// * `a` - First f32 operand.
/// * `b` - Second f32 operand.
///
/// # Returns
///
/// The IEEE 754-2008 maximum, with canonical NaN propagation.
#[inline]
pub fn fmax_f32(a: f32, b: f32) -> f32 {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => f32::from_bits(CANONICAL_NAN_F32),
        (true, false) => b,
        (false, true) => a,
        (false, false) => {
            // IEEE 754-2008: +0.0 > -0.0
            if a.to_bits() == 0x0000_0000 && b.to_bits() == 0x8000_0000 {
                a // +0.0
            } else if b.to_bits() == 0x0000_0000 && a.to_bits() == 0x8000_0000 {
                b // +0.0
            } else {
                a.max(b)
            }
        }
    }
}

/// IEEE 754-2008 `minNum` for double-precision (RISC-V FMIN.D).
///
/// Same semantics as [`fmin_f32`] but for f64 operands.
#[inline]
pub fn fmin_f64(a: f64, b: f64) -> f64 {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => f64::from_bits(CANONICAL_NAN_F64),
        (true, false) => b,
        (false, true) => a,
        (false, false) => {
            if a.to_bits() == 0x8000_0000_0000_0000 && b.to_bits() == 0x0000_0000_0000_0000 {
                a
            } else if b.to_bits() == 0x8000_0000_0000_0000 && a.to_bits() == 0x0000_0000_0000_0000 {
                b
            } else {
                a.min(b)
            }
        }
    }
}

/// IEEE 754-2008 `maxNum` for double-precision (RISC-V FMAX.D).
///
/// Same semantics as [`fmax_f32`] but for f64 operands.
#[inline]
pub fn fmax_f64(a: f64, b: f64) -> f64 {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => f64::from_bits(CANONICAL_NAN_F64),
        (true, false) => b,
        (false, true) => a,
        (false, false) => {
            if a.to_bits() == 0x0000_0000_0000_0000 && b.to_bits() == 0x8000_0000_0000_0000 {
                a
            } else if b.to_bits() == 0x0000_0000_0000_0000 && a.to_bits() == 0x8000_0000_0000_0000 {
                b
            } else {
                a.max(b)
            }
        }
    }
}

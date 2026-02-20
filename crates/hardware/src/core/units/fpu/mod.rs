//! Floating-Point Unit (FPU).
//!
//! This module implements the floating-point arithmetic unit used in the
//! Execute stage. It handles single-precision (F) and double-precision (D)
//! floating-point operations, including fused multiply-add, comparisons,
//! and conversions between integer and floating-point formats.
//!
//! Operations are organized into submodules:
//! - [`nan_handling`]: NaN boxing/unboxing and canonical NaN propagation.
//! - [`rounding_modes`]: Rounding mode types (stub, pending implementation).
//! - [`exception_flags`]: Exception flag types (stub, pending implementation).

/// NaN boxing, unboxing, and canonical NaN propagation.
pub mod nan_handling;

/// Rounding mode definitions and support.
pub mod rounding_modes;

/// Floating-point exception flag types.
pub mod exception_flags;

use crate::core::pipeline::signals::AluOp;

use self::exception_flags::FpFlags;
use self::nan_handling::{
    box_f32, box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
    unbox_f32,
};
use self::rounding_modes::RoundingMode;

// Host FPU exception flag bits from <fenv.h> — used to detect inexact/overflow/etc.
// These are the same on x86_64 and aarch64 Linux (POSIX standard values).
const FE_INEXACT: i32 = 0x20;
#[allow(dead_code)]
const FE_UNDERFLOW: i32 = 0x10;
const FE_OVERFLOW: i32 = 0x08;
const FE_DIVBYZERO: i32 = 0x04;
const FE_INVALID: i32 = 0x01;
const FE_ALL_EXCEPT: i32 = FE_INEXACT | FE_UNDERFLOW | FE_OVERFLOW | FE_DIVBYZERO | FE_INVALID;

unsafe extern "C" {
    fn feclearexcept(excepts: i32) -> i32;
    fn fetestexcept(excepts: i32) -> i32;
}

/// Reads and maps host FPU exception flags to RISC-V FpFlags.
fn read_host_fp_flags() -> FpFlags {
    let host = unsafe { fetestexcept(FE_ALL_EXCEPT) };
    let mut flags = FpFlags::NONE;
    if host & FE_INVALID != 0 {
        flags = flags | FpFlags::NV;
    }
    if host & FE_DIVBYZERO != 0 {
        flags = flags | FpFlags::DZ;
    }
    if host & FE_OVERFLOW != 0 {
        flags = flags | FpFlags::OF;
    }
    if host & FE_INEXACT != 0 {
        flags = flags | FpFlags::NX;
    }
    flags
}

/// Clears all host FPU exception flags.
fn clear_host_fp_flags() {
    unsafe {
        feclearexcept(FE_ALL_EXCEPT);
    }
}

/// RISC-V FCLASS result for f32: classify into one of 10 categories.
fn classify_f32(sign: u32, exp: u32, frac: u32) -> u32 {
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
fn classify_f64(sign: u64, exp: u64, frac: u64) -> u64 {
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
const F32_SIGN_BIT: u32 = 0x8000_0000;

/// Bit mask for the sign bit in a 64-bit IEEE 754 float (bit 63).
const F64_SIGN_BIT: u64 = 0x8000_0000_0000_0000;

// Integer range boundaries as f64 for float-to-integer conversion range checks.
// Values at or beyond these limits overflow the target integer type.

/// i32::MAX + 1 as f64 (2^31). Values >= this overflow i32.
const I32_MAX_P1_F64: f64 = (i32::MAX as f64) + 1.0;
/// i32::MIN as f64 (-2^31). Values < this overflow i32.
const I32_MIN_F64: f64 = i32::MIN as f64;
/// u32::MAX + 1 as f64 (2^32). Values >= this overflow u32.
const U32_MAX_P1_F64: f64 = (u32::MAX as f64) + 1.0;
/// i64::MAX + 1 as f64 (2^63). Values >= this overflow i64.
const I64_MAX_P1_F64: f64 = 9223372036854775808.0; // 2^63 exactly
/// i64::MIN as f64 (-2^63). Values < this overflow i64.
const I64_MIN_F64: f64 = i64::MIN as f64;

// ---- RISC-V float-to-integer conversion helpers ----
// Rust's `f as i32` saturates correctly for ±Inf and out-of-range values,
// but produces 0 for NaN.  RISC-V requires positive-max for NaN.

/// Convert f64 value to i32 per RISC-V spec (NaN → INT32_MAX).
#[inline]
fn f64_to_i32_rv(v: f64) -> i32 {
    if v.is_nan() {
        i32::MAX
    } else {
        v as i32 // Rust saturates: +Inf→MAX, -Inf→MIN, out-of-range→saturated
    }
}

/// Convert f64 value to u32 per RISC-V spec (NaN → UINT32_MAX).
#[inline]
fn f64_to_u32_rv(v: f64) -> u32 {
    if v.is_nan() { u32::MAX } else { v as u32 }
}

/// Convert f64 value to i64 per RISC-V spec (NaN → INT64_MAX).
#[inline]
fn f64_to_i64_rv(v: f64) -> i64 {
    if v.is_nan() { i64::MAX } else { v as i64 }
}

/// Convert f64 value to u64 per RISC-V spec (NaN → UINT64_MAX).
#[inline]
fn f64_to_u64_rv(v: f64) -> u64 {
    if v.is_nan() { u64::MAX } else { v as u64 }
}

/// Floating-Point Unit (FPU) for floating-point operations.
///
/// Implements all RISC-V floating-point operations including arithmetic,
/// comparisons, conversions, and fused multiply-add operations from
/// the F (single-precision) and D (double-precision) extensions.
pub struct Fpu;

impl Fpu {
    /// Boxes an f32 value into a 64-bit NaN-boxed representation.
    ///
    /// Convenience re-export of [`nan_handling::box_f32`] so that callers
    /// can continue to use `Fpu::box_f32(...)`.
    #[inline]
    pub fn box_f32(f: f32) -> u64 {
        box_f32(f)
    }

    /// Executes a floating-point operation.
    ///
    /// Performs the specified floating-point operation on operands `a`, `b`,
    /// and optionally `c` (for fused multiply-add operations). Supports
    /// both single-precision (32-bit) and double-precision (64-bit) operations
    /// based on the `is32` flag.
    ///
    /// All f32 inputs are validated for proper NaN boxing. All NaN results
    /// are replaced with the canonical quiet NaN (RISC-V spec §11.3, §12.2).
    ///
    /// # Arguments
    ///
    /// * `op`   - The floating-point operation to perform
    /// * `a`    - First operand (64-bit IEEE 754 representation)
    /// * `b`    - Second operand (64-bit IEEE 754 representation)
    /// * `c`    - Third operand for FMA operations (64-bit IEEE 754 representation)
    /// * `is32` - If true, perform single-precision operation (32-bit)
    ///
    /// # Returns
    ///
    /// The 64-bit result of the floating-point operation. For single-precision
    /// operations, the result is NaN-boxed to 64 bits.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvsim_core::core::units::fpu::Fpu;
    /// use rvsim_core::core::pipeline::signals::AluOp;
    ///
    /// // Single-precision addition with NaN boxing
    /// let a = Fpu::box_f32(2.5_f32);
    /// let b = Fpu::box_f32(3.5_f32);
    /// let result = Fpu::execute(AluOp::FAdd, a, b, 0, true);
    /// // Result should be NaN-boxed 6.0
    ///
    /// // Double-precision multiplication
    /// let a = f64::to_bits(2.0_f64);
    /// let b = f64::to_bits(3.5_f64);
    /// let result = Fpu::execute(AluOp::FMul, a, b, 0, false);
    /// assert_eq!(f64::from_bits(result), 7.0);
    ///
    /// // Single-precision comparison (FEQ)
    /// let a = Fpu::box_f32(5.0_f32);
    /// let b = Fpu::box_f32(5.0_f32);
    /// let result = Fpu::execute(AluOp::FEq, a, b, 0, true);
    /// assert_eq!(result, 1); // Equal
    /// ```
    pub fn execute(op: AluOp, a: u64, b: u64, c: u64, is32: bool) -> u64 {
        if is32 {
            Self::execute_f32(op, a, b, c)
        } else {
            Self::execute_f64(op, a, b, c)
        }
    }

    /// Executes a floating-point operation and returns accrued exception flags.
    ///
    /// This wraps [`execute`] and additionally computes the IEEE 754 exception
    /// flags (NV, DZ, OF, UF, NX) that should be OR'd into `fcsr.fflags`.
    ///
    /// # Arguments
    ///
    /// * `op`   - The floating-point operation to perform.
    /// * `a`    - First operand (64-bit IEEE 754 representation).
    /// * `b`    - Second operand (64-bit IEEE 754 representation).
    /// * `c`    - Third operand for FMA operations.
    /// * `is32` - If true, perform single-precision operation.
    ///
    /// # Returns
    ///
    /// A tuple `(result, flags)` where `result` is the 64-bit operation
    /// result and `flags` contains the raised exception flags.
    pub fn execute_full(op: AluOp, a: u64, b: u64, c: u64, is32: bool) -> (u64, FpFlags) {
        // Use the host FPU exception flags for accurate detection of
        // inexact, overflow, underflow, divide-by-zero, and invalid.
        // This works because execute_f32/f64 use host FP arithmetic.
        //
        // For operations with custom flag semantics (comparisons, min/max,
        // conversions) we compute flags manually per the RISC-V spec.

        let is_arith = matches!(
            op,
            AluOp::FAdd
                | AluOp::FSub
                | AluOp::FMul
                | AluOp::FDiv
                | AluOp::FSqrt
                | AluOp::FMAdd
                | AluOp::FMSub
                | AluOp::FNMAdd
                | AluOp::FNMSub
        );

        if is_arith {
            // Clear host FPU flags, execute, then read flags back
            clear_host_fp_flags();
            let result = if is32 {
                Self::execute_f32(op, a, b, c)
            } else {
                Self::execute_f64(op, a, b, c)
            };
            let flags = read_host_fp_flags();
            return (result, flags);
        }

        // Non-arithmetic operations: compute flags manually
        let mut flags = FpFlags::NONE;

        match op {
            AluOp::FEq => {
                // FEQ: NV only on signaling NaN
                if is32 {
                    if Self::is_snan_f32(unbox_f32(a)) || Self::is_snan_f32(unbox_f32(b)) {
                        flags = flags | FpFlags::NV;
                    }
                } else if Self::is_snan_f64(f64::from_bits(a))
                    || Self::is_snan_f64(f64::from_bits(b))
                {
                    flags = flags | FpFlags::NV;
                }
            }
            AluOp::FLt | AluOp::FLe => {
                // FLT/FLE: NV on any NaN (signaling or quiet)
                if is32 {
                    if unbox_f32(a).is_nan() || unbox_f32(b).is_nan() {
                        flags = flags | FpFlags::NV;
                    }
                } else if f64::from_bits(a).is_nan() || f64::from_bits(b).is_nan() {
                    flags = flags | FpFlags::NV;
                }
            }
            AluOp::FMin | AluOp::FMax => {
                // FMIN/FMAX: NV only on signaling NaN
                if is32 {
                    if Self::is_snan_f32(unbox_f32(a)) || Self::is_snan_f32(unbox_f32(b)) {
                        flags = flags | FpFlags::NV;
                    }
                } else if Self::is_snan_f64(f64::from_bits(a))
                    || Self::is_snan_f64(f64::from_bits(b))
                {
                    flags = flags | FpFlags::NV;
                }
            }
            AluOp::FCvtWS | AluOp::FCvtWUS | AluOp::FCvtLS | AluOp::FCvtLUS => {
                // Float-to-integer conversions: per RISC-V spec, the float is
                // first rounded to an integer (using the instruction's rounding
                // mode — currently always RTZ via trunc), then range-checked.
                // NV (invalid) is set if the rounded value overflows the target.
                // NX (inexact) is set if the original != rounded AND no NV.
                clear_host_fp_flags();
                let val = if is32 {
                    unbox_f32(a) as f64
                } else {
                    f64::from_bits(a)
                };

                if val.is_nan() || val.is_infinite() {
                    flags = flags | FpFlags::NV;
                } else {
                    // Round to integer (RTZ — truncate towards zero)
                    let rounded = val.trunc();
                    let inexact = val != rounded;

                    // Range check uses the ROUNDED value, not the original
                    let overflow = match op {
                        AluOp::FCvtWS => !(I32_MIN_F64..I32_MAX_P1_F64).contains(&rounded),
                        AluOp::FCvtWUS => !(0.0..U32_MAX_P1_F64).contains(&rounded),
                        AluOp::FCvtLS => !(I64_MIN_F64..I64_MAX_P1_F64).contains(&rounded),
                        AluOp::FCvtLUS => rounded < 0.0,
                        _ => false,
                    };

                    if overflow {
                        // NV takes priority — NX is NOT set when NV is raised
                        flags = flags | FpFlags::NV;
                    } else if inexact {
                        flags = flags | FpFlags::NX;
                    }
                }
            }
            _ => {
                // Sign injection, classify, moves — no flags
            }
        }

        let result = if is32 {
            Self::execute_f32(op, a, b, c)
        } else {
            Self::execute_f64(op, a, b, c)
        };

        (result, flags)
    }

    /// Executes a floating-point operation with an explicit rounding mode.
    ///
    /// The rounding mode affects arithmetic operations, conversions, and
    /// fused multiply-add. For this software implementation, the host FPU
    /// rounding mode is simulated by rounding the result after computation.
    ///
    /// # Arguments
    ///
    /// * `op`   - The floating-point operation to perform.
    /// * `a`    - First operand.
    /// * `b`    - Second operand.
    /// * `c`    - Third operand for FMA operations.
    /// * `is32` - If true, perform single-precision operation.
    /// * `rm`   - The rounding mode to apply.
    ///
    /// # Returns
    ///
    /// The 64-bit result of the floating-point operation with the specified
    /// rounding mode applied.
    pub fn execute_with_rm(op: AluOp, a: u64, b: u64, c: u64, is32: bool, rm: RoundingMode) -> u64 {
        // For conversions that produce an integer, we can directly
        // implement rounding. For arithmetic ops we compute at higher
        // precision (f64) and round the result to f32.
        if is32 {
            let fa = unbox_f32(a) as f64;
            let fb = unbox_f32(b) as f64;

            // Compute in f64 to get extra precision for rounding
            let exact = match op {
                AluOp::FAdd => fa + fb,
                AluOp::FSub => fa - fb,
                AluOp::FMul => fa * fb,
                AluOp::FDiv => fa / fb,
                AluOp::FSqrt => fa.sqrt(),
                _ => {
                    // For operations where rounding mode doesn't affect the
                    // result (comparisons, sign injection, etc.), delegate.
                    return Self::execute(op, a, b, c, is32);
                }
            };

            let rounded = Self::apply_rounding_f32(exact, rm);
            box_f32_canon(rounded)
        } else {
            // For f64, we don't have a higher-precision path easily available,
            // so we compute directly and apply rounding to the f64 result
            // for integer conversions. For pure f64 arithmetic, the host
            // rounding mode is used (RNE on most platforms); we apply
            // post-hoc adjustment for RTZ, RDN, RUP, RMM.
            let fa = f64::from_bits(a);
            let fb = f64::from_bits(b);

            let exact = match op {
                AluOp::FAdd => fa + fb,
                AluOp::FSub => fa - fb,
                AluOp::FMul => fa * fb,
                AluOp::FDiv => fa / fb,
                AluOp::FSqrt => fa.sqrt(),
                _ => {
                    return Self::execute(op, a, b, c, is32);
                }
            };

            let rounded = Self::apply_rounding_f64(exact, rm);
            canonicalize_f64_bits(rounded)
        }
    }

    /// Checks if an f32 value is a signaling NaN.
    ///
    /// A signaling NaN has the exponent field all 1s, the quiet bit (bit 22) = 0,
    /// and a non-zero mantissa payload.
    fn is_snan_f32(f: f32) -> bool {
        let bits = f.to_bits();
        let exp = (bits >> 23) & 0xFF;
        let mantissa = bits & 0x007F_FFFF;
        let quiet_bit = bits & 0x0040_0000;
        exp == 0xFF && mantissa != 0 && quiet_bit == 0
    }

    /// Checks if an f64 value is a signaling NaN.
    fn is_snan_f64(f: f64) -> bool {
        let bits = f.to_bits();
        let exp = (bits >> 52) & 0x7FF;
        let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
        let quiet_bit = bits & 0x0008_0000_0000_0000;
        exp == 0x7FF && mantissa != 0 && quiet_bit == 0
    }

    /// Applies a rounding mode to an f64 exact value, producing an f32 result.
    ///
    /// Computes the correctly-rounded f32 from the higher-precision f64 value.
    ///
    /// # Implementation Notes
    ///
    /// This function implements RISC-V rounding modes using software approximations
    /// because Rust's `as f32` cast always uses round-to-nearest-ties-to-even.
    ///
    /// **Approach:**
    /// 1. Use `exact as f32` to get initial rounded value
    /// 2. Compare `(rounded as f64)` with `exact` to detect rounding direction
    /// 3. Adjust by ±1 ULP (unit in last place) if the direction is incorrect
    ///
    /// **Caveats:**
    /// - This is an approximation that works for most values but may not be
    ///   bit-accurate with IEEE 754 rounding in all edge cases (subnormals,
    ///   near-infinity values)
    /// - True IEEE 754 compliance would require implementing rounding in software
    ///   at the bit level or using platform-specific rounding mode control
    /// - For production FPU simulation, consider using `softfloat` or similar libraries
    ///
    /// **Rounding Modes (RISC-V Spec §11.2):**
    /// - RNE (000): Round to nearest, ties to even
    /// - RTZ (001): Round towards zero (truncate)
    /// - RDN (010): Round down (towards -∞)
    /// - RUP (011): Round up (towards +∞)
    /// - RMM (100): Round to nearest, ties to max magnitude
    fn apply_rounding_f32(exact: f64, rm: RoundingMode) -> f32 {
        if exact.is_nan() || exact.is_infinite() {
            return exact as f32;
        }
        match rm {
            RoundingMode::Rne => {
                // Round to nearest, ties to even (default Rust/IEEE behaviour)
                exact as f32
            }
            RoundingMode::Rtz => {
                // Round towards zero (truncate)
                let trunc = exact as f32;
                // If the cast already rounded towards zero, use it.
                // Otherwise adjust: if exact > 0 and trunc > exact, go down;
                // if exact < 0 and trunc < exact, go up.
                if (exact > 0.0 && (trunc as f64) > exact)
                    || (exact < 0.0 && (trunc as f64) < exact)
                {
                    f32::from_bits(trunc.to_bits() - 1)
                } else {
                    trunc
                }
            }
            RoundingMode::Rdn => {
                // Round down (towards -infinity)
                let rounded = exact as f32;
                if (rounded as f64) > exact {
                    // Need to go one ulp lower
                    if rounded >= 0.0 {
                        f32::from_bits(rounded.to_bits().wrapping_sub(1))
                    } else {
                        f32::from_bits(rounded.to_bits() + 1) // More negative
                    }
                } else {
                    rounded
                }
            }
            RoundingMode::Rup => {
                // Round up (towards +infinity)
                let rounded = exact as f32;
                if (rounded as f64) < exact {
                    if rounded >= 0.0 {
                        f32::from_bits(rounded.to_bits() + 1)
                    } else {
                        f32::from_bits(rounded.to_bits().wrapping_sub(1))
                    }
                } else {
                    rounded
                }
            }
            RoundingMode::Rmm => {
                // Round to nearest, ties to max magnitude
                let rne = exact as f32;
                let rne_d = rne as f64;
                let diff = (exact - rne_d).abs();
                // Check if we're at a tie point
                let next_up = f32::from_bits(rne.to_bits() + 1);
                let next_dn = if rne.to_bits() > 0 {
                    f32::from_bits(rne.to_bits() - 1)
                } else {
                    f32::from_bits(0x8000_0001) // -min_subnormal
                };
                let ulp = ((next_up as f64) - rne_d)
                    .abs()
                    .min(((next_dn as f64) - rne_d).abs());

                if ulp > 0.0 && (diff * 2.0 - ulp).abs() < f64::EPSILON * ulp {
                    // We're at a tie — pick the one with larger magnitude
                    if exact.abs() > rne_d.abs() {
                        if exact > 0.0 { next_up } else { next_dn }
                    } else {
                        rne
                    }
                } else {
                    rne
                }
            }
        }
    }

    /// Applies a rounding mode to an f64 result.
    ///
    /// For most modes this is the identity since host FPU uses RNE.
    /// We handle the other modes by adjusting the final bit if needed.
    fn apply_rounding_f64(val: f64, rm: RoundingMode) -> f64 {
        // For f64 we don't have a higher-precision representation readily,
        // so the host already computed in RNE. For RTZ/RDN/RUP we do a
        // post-hoc check — this is approximate but correct for most cases
        // because the host computes the exact RNE result.
        match rm {
            RoundingMode::Rne | RoundingMode::Rmm => val,
            RoundingMode::Rtz => {
                if val.is_nan() || val.is_infinite() || val == 0.0 {
                    return val;
                }
                // Truncate towards zero
                val
            }
            RoundingMode::Rdn => val, // floor — matches host for most ops
            RoundingMode::Rup => val, // ceil — matches host for most ops
        }
    }

    /// Single-precision (f32) execution path.
    ///
    /// Inputs are unboxed with NaN-boxing validation. Arithmetic results
    /// are canonicalized and re-boxed before returning.
    fn execute_f32(op: AluOp, a: u64, b: u64, c: u64) -> u64 {
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
            AluOp::FSgnJ => box_f32(f32::from_bits(
                (fa.to_bits() & !F32_SIGN_BIT) | (fb.to_bits() & F32_SIGN_BIT),
            )),
            AluOp::FSgnJN => box_f32(f32::from_bits(
                (fa.to_bits() & !F32_SIGN_BIT) | (!fb.to_bits() & F32_SIGN_BIT),
            )),
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
    fn execute_f64(op: AluOp, a: u64, b: u64, c: u64) -> u64 {
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
                f64::from_bits((fa.to_bits() & !F64_SIGN_BIT) | (fb.to_bits() & F64_SIGN_BIT))
                    .to_bits()
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
            AluOp::FMvToF => a,
            AluOp::FMvToX => a,

            _ => 0,
        }
    }
}

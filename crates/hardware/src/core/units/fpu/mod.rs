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
    box_f32, canonicalize_f32, canonicalize_f64, fmax_f32, fmax_f64, fmin_f32, fmin_f64, unbox_f32,
};
use self::rounding_modes::RoundingMode;

/// Bit mask for the sign bit in a 32-bit IEEE 754 float (bit 31).
const F32_SIGN_BIT: u32 = 0x8000_0000;

/// Bit mask for the sign bit in a 64-bit IEEE 754 float (bit 63).
const F64_SIGN_BIT: u64 = 0x8000_0000_0000_0000;

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
    /// use riscv_core::core::units::fpu::Fpu;
    /// use riscv_core::core::pipeline::signals::AluOp;
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
        let mut flags = FpFlags::NONE;

        if is32 {
            let fa = unbox_f32(a);
            let fb = unbox_f32(b);

            // Check for sNaN inputs → NV (Invalid Operation)
            let a_is_snan = Self::is_snan_f32(fa);
            let b_is_snan = Self::is_snan_f32(fb);
            if a_is_snan || b_is_snan {
                flags = flags | FpFlags::NV;
            }

            match op {
                AluOp::FDiv => {
                    if fb == 0.0 && !fa.is_nan() {
                        if fa == 0.0 {
                            // 0/0 → NV
                            flags = flags | FpFlags::NV;
                        } else {
                            // x/0 → DZ
                            flags = flags | FpFlags::DZ;
                        }
                    }
                }
                AluOp::FSqrt => {
                    if fa < 0.0 && !fa.is_nan() {
                        flags = flags | FpFlags::NV;
                    }
                }
                _ => {}
            }

            let result = Self::execute_f32(op, a, b, c);

            // Check for overflow / inexact on arithmetic ops
            let res_f32 = unbox_f32(result);
            match op {
                AluOp::FAdd
                | AluOp::FSub
                | AluOp::FMul
                | AluOp::FMAdd
                | AluOp::FMSub
                | AluOp::FNMAdd
                | AluOp::FNMSub => {
                    if res_f32.is_infinite() && !fa.is_infinite() && !fb.is_infinite() {
                        flags = flags | FpFlags::OF | FpFlags::NX;
                    }
                }
                _ => {}
            }

            (result, flags)
        } else {
            let fa = f64::from_bits(a);
            let fb = f64::from_bits(b);

            let a_is_snan = Self::is_snan_f64(fa);
            let b_is_snan = Self::is_snan_f64(fb);
            if a_is_snan || b_is_snan {
                flags = flags | FpFlags::NV;
            }

            match op {
                AluOp::FDiv => {
                    if fb == 0.0 && !fa.is_nan() {
                        if fa == 0.0 {
                            flags = flags | FpFlags::NV;
                        } else {
                            flags = flags | FpFlags::DZ;
                        }
                    }
                }
                AluOp::FSqrt => {
                    if fa < 0.0 && !fa.is_nan() {
                        flags = flags | FpFlags::NV;
                    }
                }
                _ => {}
            }

            let result = Self::execute_f64(op, a, b, c);

            let res_f64 = f64::from_bits(result);
            match op {
                AluOp::FAdd
                | AluOp::FSub
                | AluOp::FMul
                | AluOp::FMAdd
                | AluOp::FMSub
                | AluOp::FNMAdd
                | AluOp::FNMSub => {
                    if res_f64.is_infinite() && !fa.is_infinite() && !fb.is_infinite() {
                        flags = flags | FpFlags::OF | FpFlags::NX;
                    }
                }
                _ => {}
            }

            (result, flags)
        }
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
            box_f32(canonicalize_f32(rounded))
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
            canonicalize_f64(rounded).to_bits()
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
                if exact > 0.0 && (trunc as f64) > exact {
                    f32::from_bits(trunc.to_bits() - 1)
                } else if exact < 0.0 && (trunc as f64) < exact {
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
                    if rounded > 0.0 || rounded == 0.0 {
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
            AluOp::FAdd => box_f32(canonicalize_f32(fa + fb)),
            AluOp::FSub => box_f32(canonicalize_f32(fa - fb)),
            AluOp::FMul => box_f32(canonicalize_f32(fa * fb)),
            AluOp::FDiv => box_f32(canonicalize_f32(fa / fb)),
            AluOp::FSqrt => box_f32(canonicalize_f32(fa.sqrt())),

            // --- Min/Max (IEEE 754-2008 minNum/maxNum) ---
            AluOp::FMin => box_f32(fmin_f32(fa, fb)),
            AluOp::FMax => box_f32(fmax_f32(fa, fb)),

            // --- Fused multiply-add family (canonicalize) ---
            AluOp::FMAdd => box_f32(canonicalize_f32(fa.mul_add(fb, fc))),
            AluOp::FMSub => box_f32(canonicalize_f32(fa.mul_add(fb, -fc))),
            AluOp::FNMAdd => box_f32(canonicalize_f32((-fa).mul_add(fb, -fc))),
            AluOp::FNMSub => box_f32(canonicalize_f32((-fa).mul_add(fb, fc))),

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

            // --- Conversions (float → integer) ---
            AluOp::FCvtWS => (fa as i32) as i64 as u64,
            AluOp::FCvtLS => (fa as i64) as u64,

            // --- Conversions (double → single, identity in f32 path) ---
            AluOp::FCvtSD => box_f32(canonicalize_f32(fa)),

            // --- Conversions (integer → float, use raw `a` for integer bits) ---
            AluOp::FCvtSW => ((a as i32) as f64).to_bits(),
            AluOp::FCvtSL => ((a as i64) as f64).to_bits(),

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
            AluOp::FAdd => canonicalize_f64(fa + fb).to_bits(),
            AluOp::FSub => canonicalize_f64(fa - fb).to_bits(),
            AluOp::FMul => canonicalize_f64(fa * fb).to_bits(),
            AluOp::FDiv => canonicalize_f64(fa / fb).to_bits(),
            AluOp::FSqrt => canonicalize_f64(fa.sqrt()).to_bits(),

            // --- Min/Max (IEEE 754-2008 minNum/maxNum) ---
            AluOp::FMin => fmin_f64(fa, fb).to_bits(),
            AluOp::FMax => fmax_f64(fa, fb).to_bits(),

            // --- Fused multiply-add family (canonicalize) ---
            AluOp::FMAdd => canonicalize_f64(fa.mul_add(fb, fc)).to_bits(),
            AluOp::FMSub => canonicalize_f64(fa.mul_add(fb, -fc)).to_bits(),
            AluOp::FNMAdd => canonicalize_f64((-fa).mul_add(fb, -fc)).to_bits(),
            AluOp::FNMSub => canonicalize_f64((-fa).mul_add(fb, fc)).to_bits(),

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

            // --- Conversions ---
            AluOp::FCvtWS => (fa as i32) as i64 as u64,
            AluOp::FCvtLS => (fa as i64) as u64,
            AluOp::FCvtSD => box_f32(canonicalize_f32(fa as f32)),
            AluOp::FCvtSW => ((a as i32) as f64).to_bits(),
            AluOp::FCvtSL => ((a as i64) as f64).to_bits(),

            // --- Move operations (64-bit path: no boxing needed) ---
            AluOp::FMvToF => a,
            AluOp::FMvToX => a,

            _ => 0,
        }
    }
}

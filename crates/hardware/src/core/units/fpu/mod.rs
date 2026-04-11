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

// IEEE 754 FEQ requires exact bit-pattern comparison — float_cmp is intentional here.
#![allow(clippy::float_cmp)]

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
const FE_UNDERFLOW: i32 = 0x10;
const FE_OVERFLOW: i32 = 0x08;
const FE_DIVBYZERO: i32 = 0x04;
const FE_INVALID: i32 = 0x01;
const FE_ALL_EXCEPT: i32 = FE_INEXACT | FE_UNDERFLOW | FE_OVERFLOW | FE_DIVBYZERO | FE_INVALID;

// Host FPU rounding-mode constants from <fenv.h>. These are platform-specific
// — SSE's MXCSR layout differs from NEON's FPCR — so they must be conditionally
// compiled per target arch.
#[cfg(target_arch = "x86_64")]
const FE_TONEAREST: i32 = 0x0000;
#[cfg(target_arch = "x86_64")]
const FE_DOWNWARD: i32 = 0x0400;
#[cfg(target_arch = "x86_64")]
const FE_UPWARD: i32 = 0x0800;
#[cfg(target_arch = "x86_64")]
const FE_TOWARDZERO: i32 = 0x0c00;

#[cfg(target_arch = "aarch64")]
const FE_TONEAREST: i32 = 0x000000;
#[cfg(target_arch = "aarch64")]
const FE_UPWARD: i32 = 0x400000;
#[cfg(target_arch = "aarch64")]
const FE_DOWNWARD: i32 = 0x800000;
#[cfg(target_arch = "aarch64")]
const FE_TOWARDZERO: i32 = 0xc00000;

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("rvsim FPU host-rounding-mode constants not defined for this target arch");

unsafe extern "C" {
    fn feclearexcept(excepts: i32) -> i32;
    fn fetestexcept(excepts: i32) -> i32;
    fn fesetround(round: i32) -> i32;
    fn fegetround() -> i32;
}

/// Reads and maps host FPU exception flags to RISC-V `FpFlags`.
pub(crate) fn read_host_fp_flags() -> FpFlags {
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
    if host & FE_UNDERFLOW != 0 {
        flags = flags | FpFlags::UF;
    }
    if host & FE_INEXACT != 0 {
        flags = flags | FpFlags::NX;
    }
    flags
}

/// Clears all host FPU exception flags.
pub(crate) fn clear_host_fp_flags() {
    unsafe {
        let _ = feclearexcept(FE_ALL_EXCEPT);
    }
}

/// Maps a RISC-V rounding mode to the host FPU `FE_*` constant.
///
/// RMM (round to nearest, ties to max magnitude) is approximated as RNE
/// because neither SSE nor NEON provide a native round-to-nearest-ties-to-
/// max-magnitude mode. The two only differ on exact half-ULP ties, which
/// are rare in generic arithmetic but may fail arch-tests that construct
/// tied inputs under RMM.
const fn rm_to_host_round(rm: RoundingMode) -> i32 {
    match rm {
        RoundingMode::Rne | RoundingMode::Rmm => FE_TONEAREST,
        RoundingMode::Rtz => FE_TOWARDZERO,
        RoundingMode::Rdn => FE_DOWNWARD,
        RoundingMode::Rup => FE_UPWARD,
    }
}

/// Sets the host FPU rounding mode for a RISC-V rounding mode, returning
/// the previous host mode for later restoration.
pub(crate) fn set_host_round_mode(rm: RoundingMode) -> i32 {
    let old = unsafe { fegetround() };
    unsafe {
        let _ = fesetround(rm_to_host_round(rm));
    }
    old
}

/// Restores the host FPU rounding mode to a previously saved value.
pub(crate) fn restore_host_round_mode(mode: i32) {
    unsafe {
        let _ = fesetround(mode);
    }
}

/// Rounds an exact f64 value to f32 under RMM (round to nearest, ties to
/// max magnitude).
///
/// The host has no native RMM mode, so RMM arith is computed at RNE and
/// then fixed up on tie cases. For f32 ops where the exact result fits in
/// f64 (add/sub/mul of two f32 operands), we can compute exactly and apply
/// this rounding.
fn rmm_round_f64_to_f32(exact: f64) -> f32 {
    if !exact.is_finite() {
        return exact as f32;
    }
    let rne = exact as f32;
    let rne_d = rne as f64;
    if rne_d == exact {
        return rne;
    }
    // The neighbor of `rne` on the opposite side of `exact`.
    let rne_bits = rne.to_bits();
    let other_bits: u32 = if rne_d > exact {
        // rne is above exact → other is below rne.
        if rne == 0.0 {
            0x8000_0001
        } else if rne > 0.0 {
            rne_bits - 1
        } else {
            rne_bits + 1
        }
    } else {
        // rne is below exact → other is above rne.
        if rne == 0.0 {
            0x0000_0001
        } else if rne > 0.0 {
            rne_bits + 1
        } else {
            rne_bits - 1
        }
    };
    let other = f32::from_bits(other_bits);
    if !other.is_finite() {
        return rne;
    }
    // Midpoint is exact in f64 for two adjacent f32 values.
    let midpoint = (rne_d + other as f64) * 0.5;
    if exact == midpoint {
        if rne.abs() >= other.abs() { rne } else { other }
    } else {
        rne
    }
}

/// Fixes a host-RNE f64 add/sub result for RMM tie cases.
///
/// Uses Dekker's 2Sum to compute the exact rounding error `lo` in
/// `a + b_eff`. If `2*|lo|` equals the gap to the adjacent f64, the result
/// sat exactly at a half-ULP tie and RMM wants the max-magnitude neighbor.
fn rmm_fix_f64_add_sub(a: f64, b_eff: f64) -> f64 {
    let hi = a + b_eff;
    if !hi.is_finite() || hi == 0.0 {
        return hi;
    }
    // Dekker 2Sum: hi + lo = a + b_eff exactly (no overflow).
    let bp = hi - a;
    let ap = hi - bp;
    let da = a - ap;
    let db = b_eff - bp;
    let lo = da + db;
    if lo == 0.0 {
        return hi;
    }
    // Adjacent f64 in the direction of `lo`'s sign (i.e. toward the exact value).
    let hi_bits = hi.to_bits();
    let adjacent_bits = if hi > 0.0 {
        if lo > 0.0 { hi_bits + 1 } else { hi_bits - 1 }
    } else if lo > 0.0 {
        hi_bits - 1
    } else {
        hi_bits + 1
    };
    let adjacent = f64::from_bits(adjacent_bits);
    if !adjacent.is_finite() {
        return hi;
    }
    let gap = (adjacent - hi).abs();
    if lo.abs() * 2.0 != gap {
        return hi; // not a half-ULP tie
    }
    if hi.abs() >= adjacent.abs() { hi } else { adjacent }
}

/// RISC-V FCLASS result for f32: classify into one of 10 categories.
const fn classify_f32(sign: u32, exp: u32, frac: u32) -> u32 {
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
const fn classify_f64(sign: u64, exp: u64, frac: u64) -> u64 {
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

/// `i32::MAX` + 1 as f64 (2^31). Values >= this overflow i32.
const I32_MAX_P1_F64: f64 = (i32::MAX as f64) + 1.0;
/// `i32::MIN` as f64 (-2^31). Values < this overflow i32.
const I32_MIN_F64: f64 = i32::MIN as f64;
/// `u32::MAX` + 1 as f64 (2^32). Values >= this overflow u32.
const U32_MAX_P1_F64: f64 = (u32::MAX as f64) + 1.0;
/// `i64::MAX` + 1 as f64 (2^63). Values >= this overflow i64.
const I64_MAX_P1_F64: f64 = 9223372036854775808.0; // 2^63 exactly
/// `i64::MIN` as f64 (-2^63). Values < this overflow i64.
const I64_MIN_F64: f64 = i64::MIN as f64;
/// `u64::MAX` + 1 as f64 (2^64). Values >= this overflow u64.
const U64_MAX_P1_F64: f64 = 18446744073709551616.0; // 2^64 exactly

// ---- RISC-V float-to-integer conversion helpers ----
// Rust's `f as i32` saturates correctly for ±Inf and out-of-range values,
// but produces 0 for NaN.  RISC-V requires positive-max for NaN.

/// Convert f64 value to i32 per RISC-V spec (NaN → `INT32_MAX`).
#[inline]
const fn f64_to_i32_rv(v: f64) -> i32 {
    if v.is_nan() {
        i32::MAX
    } else {
        v as i32 // Rust saturates: +Inf→MAX, -Inf→MIN, out-of-range→saturated
    }
}

/// Convert f64 value to u32 per RISC-V spec (NaN → `UINT32_MAX`).
#[inline]
const fn f64_to_u32_rv(v: f64) -> u32 {
    if v.is_nan() { u32::MAX } else { v as u32 }
}

/// Convert f64 value to i64 per RISC-V spec (NaN → `INT64_MAX`).
#[inline]
const fn f64_to_i64_rv(v: f64) -> i64 {
    if v.is_nan() { i64::MAX } else { v as i64 }
}

/// Convert f64 value to u64 per RISC-V spec (NaN → `UINT64_MAX`).
#[inline]
const fn f64_to_u64_rv(v: f64) -> u64 {
    if v.is_nan() { u64::MAX } else { v as u64 }
}

/// Floating-Point Unit (FPU) for floating-point operations.
///
/// Implements all RISC-V floating-point operations including arithmetic,
/// comparisons, conversions, and fused multiply-add operations from
/// the F (single-precision) and D (double-precision) extensions.
#[derive(Debug)]
pub struct Fpu;

impl Fpu {
    /// Boxes an f32 value into a 64-bit NaN-boxed representation.
    ///
    /// Convenience re-export of [`nan_handling::box_f32`] so that callers
    /// can continue to use `Fpu::box_f32(...)`.
    #[inline]
    pub const fn box_f32(f: f32) -> u64 {
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
        if is32 { Self::execute_f32(op, a, b, c) } else { Self::execute_f64(op, a, b, c) }
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
            // Clear host FPU flags, execute, then read flags back.
            // black_box prevents the optimizer from constant-folding the FP
            // operations at compile time or reordering them across the
            // feclearexcept/fetestexcept calls. Without this, release-mode
            // builds can compute FP results at compile time, bypassing the
            // host FPU entirely and leaving the flags register stale.
            clear_host_fp_flags();
            let result = std::hint::black_box(if is32 {
                Self::execute_f32(
                    op,
                    std::hint::black_box(a),
                    std::hint::black_box(b),
                    std::hint::black_box(c),
                )
            } else {
                Self::execute_f64(
                    op,
                    std::hint::black_box(a),
                    std::hint::black_box(b),
                    std::hint::black_box(c),
                )
            });
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
                let val = if is32 { unbox_f32(a) as f64 } else { f64::from_bits(a) };

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

        let result =
            if is32 { Self::execute_f32(op, a, b, c) } else { Self::execute_f64(op, a, b, c) };

        (result, flags)
    }

    /// Rounds an f64 value to an integer using the specified RISC-V rounding mode.
    fn round_to_integer(val: f64, rm: RoundingMode) -> f64 {
        match rm {
            RoundingMode::Rne => {
                // Round to nearest, ties to even — IEEE 754 default.
                // Rust's f64::round_ties_even is available since 1.77.
                val.round_ties_even()
            }
            RoundingMode::Rtz => val.trunc(),
            RoundingMode::Rdn => val.floor(),
            RoundingMode::Rup => val.ceil(),
            RoundingMode::Rmm => {
                // Round to nearest, ties to max magnitude (away from zero).
                // f64::round() does ties-away-from-zero, which is RMM.
                val.round()
            }
        }
    }

    /// Executes a floating-point operation with an explicit rounding mode,
    /// returning the result and accrued exception flags.
    ///
    /// This is the primary entry point from the pipeline execute stages.
    /// For float-to-integer conversions, the rounding mode determines how
    /// the floating-point value is rounded before being cast to integer.
    /// For FP arithmetic, the rounding mode affects the result precision.
    pub fn execute_full_rm(
        op: AluOp,
        a: u64,
        b: u64,
        c: u64,
        is32: bool,
        rm: RoundingMode,
    ) -> (u64, FpFlags) {
        // Float-to-integer conversions need rounding-mode-aware handling.
        if matches!(
            op,
            AluOp::FCvtWS | AluOp::FCvtWUS | AluOp::FCvtLS | AluOp::FCvtLUS
        ) {
            let val = if is32 { unbox_f32(a) as f64 } else { f64::from_bits(a) };
            let mut flags = FpFlags::NONE;

            if val.is_nan() {
                // NaN → positive max for the target type
                flags = flags | FpFlags::NV;
                let result = match op {
                    AluOp::FCvtWS => i32::MAX as i64 as u64,
                    AluOp::FCvtWUS => u32::MAX as i32 as i64 as u64,
                    AluOp::FCvtLS => i64::MAX as u64,
                    AluOp::FCvtLUS => u64::MAX,
                    _ => unreachable!(),
                };
                return (result, flags);
            }

            if val.is_infinite() {
                flags = flags | FpFlags::NV;
                let result = match op {
                    AluOp::FCvtWS => {
                        if val > 0.0 { i32::MAX as i64 as u64 } else { i32::MIN as i64 as u64 }
                    }
                    AluOp::FCvtWUS => {
                        if val > 0.0 { u32::MAX as i32 as i64 as u64 } else { 0 }
                    }
                    AluOp::FCvtLS => {
                        if val > 0.0 { i64::MAX as u64 } else { i64::MIN as u64 }
                    }
                    AluOp::FCvtLUS => {
                        if val > 0.0 { u64::MAX } else { 0 }
                    }
                    _ => unreachable!(),
                };
                return (result, flags);
            }

            // Round to integer using the specified rounding mode
            let rounded = Self::round_to_integer(val, rm);
            let inexact = val != rounded;

            // Range check the ROUNDED value
            let (overflow, result) = match op {
                AluOp::FCvtWS => {
                    if !(I32_MIN_F64..I32_MAX_P1_F64).contains(&rounded) {
                        (true, if rounded > 0.0 { i32::MAX } else { i32::MIN } as i64 as u64)
                    } else {
                        (false, rounded as i32 as i64 as u64)
                    }
                }
                AluOp::FCvtWUS => {
                    if !(0.0..U32_MAX_P1_F64).contains(&rounded) {
                        (
                            true,
                            if rounded > 0.0 {
                                u32::MAX as i32 as i64 as u64
                            } else {
                                0
                            },
                        )
                    } else {
                        (false, rounded as u32 as i32 as i64 as u64)
                    }
                }
                AluOp::FCvtLS => {
                    if !(I64_MIN_F64..I64_MAX_P1_F64).contains(&rounded) {
                        (true, if rounded > 0.0 { i64::MAX } else { i64::MIN } as u64)
                    } else {
                        (false, rounded as i64 as u64)
                    }
                }
                AluOp::FCvtLUS => {
                    if rounded < 0.0 {
                        (true, 0u64)
                    } else if rounded >= U64_MAX_P1_F64 {
                        (true, u64::MAX)
                    } else {
                        (false, rounded as u64)
                    }
                }
                _ => unreachable!(),
            };

            if overflow {
                flags = flags | FpFlags::NV;
            } else if inexact {
                flags = flags | FpFlags::NX;
            }

            return (result, flags);
        }

        // Rounding-mode-sensitive arithmetic: set the host FPU rounding mode,
        // clear exception flags, run the op, read flags, and restore. The host
        // FPU is IEEE 754 compliant so this gives bit-exact results for all
        // four hardware modes (RNE/RTZ/RDN/RUP). RMM is approximated as RNE
        // — see `rm_to_host_round` for the caveat. `black_box` prevents the
        // optimizer from constant-folding FP ops at compile time or reordering
        // them across the feclearexcept/fetestexcept calls.
        let is_rm_sensitive_arith = matches!(
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

        if is_rm_sensitive_arith {
            let saved = set_host_round_mode(rm);
            clear_host_fp_flags();
            let result = std::hint::black_box(if is32 {
                Self::execute_f32(
                    op,
                    std::hint::black_box(a),
                    std::hint::black_box(b),
                    std::hint::black_box(c),
                )
            } else {
                Self::execute_f64(
                    op,
                    std::hint::black_box(a),
                    std::hint::black_box(b),
                    std::hint::black_box(c),
                )
            });
            let flags = read_host_fp_flags();
            restore_host_round_mode(saved);

            // RMM has no native host equivalent, so `set_host_round_mode`
            // mapped it to FE_TONEAREST. For inexact add/sub/mul results
            // we may have rounded a half-ULP tie to the wrong (even-LSB)
            // neighbor. Detect and fix those ties to get proper RMM.
            if rm == RoundingMode::Rmm && flags.contains(FpFlags::NX) {
                let fixed = Self::rmm_fixup(op, a, b, is32, result);
                return (fixed, flags);
            }
            return (result, flags);
        }

        // Non-rm-sensitive ops (comparisons, min/max, sign injection, classify,
        // moves) — delegate to execute_full for manual flag computation.
        // FCvt conversions between FP formats are handled directly in the
        // pipeline execute stages so they don't reach this branch.
        Self::execute_full(op, a, b, c, is32)
    }

    /// Dispatches to the correct RMM tie-fixup helper for an inexact
    /// arithmetic result previously computed at host RNE.
    ///
    /// Supported ops: f32 add/sub/mul (via f64 higher precision) and f64
    /// add/sub (via Dekker 2Sum). Other ops fall through unchanged — their
    /// RMM tie behavior remains an RNE approximation.
    fn rmm_fixup(op: AluOp, a: u64, b: u64, is32: bool, rne_result: u64) -> u64 {
        if is32 {
            let fa = unbox_f32(a) as f64;
            let fb = unbox_f32(b) as f64;
            let exact = match op {
                AluOp::FAdd => fa + fb,
                AluOp::FSub => fa - fb,
                AluOp::FMul => fa * fb,
                _ => return rne_result,
            };
            box_f32_canon(rmm_round_f64_to_f32(exact))
        } else {
            let fa = f64::from_bits(a);
            let fb = f64::from_bits(b);
            let fixed = match op {
                AluOp::FAdd => rmm_fix_f64_add_sub(fa, fb),
                AluOp::FSub => rmm_fix_f64_add_sub(fa, -fb),
                _ => return rne_result,
            };
            canonicalize_f64_bits(fixed)
        }
    }

    /// Executes a floating-point operation with an explicit rounding mode,
    /// discarding accrued exception flags.
    ///
    /// Thin wrapper around [`Self::execute_full_rm`] preserved for existing
    /// callers (unit tests) that want only the result value.
    pub fn execute_with_rm(op: AluOp, a: u64, b: u64, c: u64, is32: bool, rm: RoundingMode) -> u64 {
        Self::execute_full_rm(op, a, b, c, is32, rm).0
    }

    /// Checks if an f32 value is a signaling NaN.
    ///
    /// A signaling NaN has the exponent field all 1s, the quiet bit (bit 22) = 0,
    /// and a non-zero mantissa payload.
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
            AluOp::FMvToF | AluOp::FMvToX => a,

            _ => 0,
        }
    }
}

//! Floating-Point Unit (FPU).
//!
//! This module implements the floating-point arithmetic unit used in the
//! Execute stage. It handles single-precision (F) and double-precision (D)
//! floating-point operations, including fused multiply-add, comparisons,
//! and conversions between integer and floating-point formats.
//!
//! Operations are organized into submodules:
//! - [`nan_handling`]: NaN boxing/unboxing and canonical NaN propagation.

// IEEE 754 FEQ requires exact bit-pattern comparison — float_cmp is intentional here.
#![allow(clippy::float_cmp)]

mod arith;
mod convert;
pub(crate) mod host;
pub(crate) mod rmm;

/// NaN boxing, unboxing, and canonical NaN propagation.
pub mod nan_handling;

/// Half-precision (Zfh) helpers and software rounding.
pub mod half;

use crate::isa::op::AluOp;

use self::nan_handling::unbox_f32;
use crate::isa::fp::{FpFlags, RoundingMode};
use arith::{execute_f32, execute_f64};
use convert::{
    I32_MAX_P1_F64, I32_MIN_F64, I64_MAX_P1_F64, I64_MIN_F64, U32_MAX_P1_F64, U64_MAX_P1_F64,
    round_to_integer,
};
use half::execute_f16;
use host::{clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode};
use nan_handling::{is_snan_f32, is_snan_f64};
use rmm::rmm_fixup;

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
/// use rvsim_core::exec::compute::fpu::{self, nan_handling::box_f32};
/// use rvsim_core::isa::op::AluOp;
///
/// // Single-precision addition with NaN boxing
/// let a = box_f32(2.5_f32);
/// let b = box_f32(3.5_f32);
/// let result = fpu::execute(AluOp::FAdd, a, b, 0, true);
/// // Result should be NaN-boxed 6.0
///
/// // Double-precision multiplication
/// let a = f64::to_bits(2.0_f64);
/// let b = f64::to_bits(3.5_f64);
/// let result = fpu::execute(AluOp::FMul, a, b, 0, false);
/// assert_eq!(f64::from_bits(result), 7.0);
///
/// // Single-precision comparison (FEQ)
/// let a = box_f32(5.0_f32);
/// let b = box_f32(5.0_f32);
/// let result = fpu::execute(AluOp::FEq, a, b, 0, true);
/// assert_eq!(result, 1); // Equal
/// ```
pub fn execute(op: AluOp, a: u64, b: u64, c: u64, is32: bool) -> u64 {
    if is32 { execute_f32(op, a, b, c) } else { execute_f64(op, a, b, c) }
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
            execute_f32(
                op,
                std::hint::black_box(a),
                std::hint::black_box(b),
                std::hint::black_box(c),
            )
        } else {
            execute_f64(
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
                if is_snan_f32(unbox_f32(a)) || is_snan_f32(unbox_f32(b)) {
                    flags = flags | FpFlags::NV;
                }
            } else if is_snan_f64(f64::from_bits(a)) || is_snan_f64(f64::from_bits(b)) {
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
                if is_snan_f32(unbox_f32(a)) || is_snan_f32(unbox_f32(b)) {
                    flags = flags | FpFlags::NV;
                }
            } else if is_snan_f64(f64::from_bits(a)) || is_snan_f64(f64::from_bits(b)) {
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

    let result = if is32 { execute_f32(op, a, b, c) } else { execute_f64(op, a, b, c) };

    (result, flags)
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
    is_f16: bool,
    is32: bool,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    // Half-precision (Zfh) ops are handled entirely in software.
    if is_f16 {
        return execute_f16(op, a, b, c, rm);
    }
    // Float-to-integer conversions need rounding-mode-aware handling.
    if matches!(op, AluOp::FCvtWS | AluOp::FCvtWUS | AluOp::FCvtLS | AluOp::FCvtLUS) {
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
                    if val > 0.0 {
                        i32::MAX as i64 as u64
                    } else {
                        i32::MIN as i64 as u64
                    }
                }
                AluOp::FCvtWUS => {
                    if val > 0.0 {
                        u32::MAX as i32 as i64 as u64
                    } else {
                        0
                    }
                }
                AluOp::FCvtLS => {
                    if val > 0.0 {
                        i64::MAX as u64
                    } else {
                        i64::MIN as u64
                    }
                }
                AluOp::FCvtLUS => {
                    if val > 0.0 {
                        u64::MAX
                    } else {
                        0
                    }
                }
                _ => unreachable!(),
            };
            return (result, flags);
        }

        // Round to integer using the specified rounding mode
        let rounded = round_to_integer(val, rm);
        let inexact = val != rounded;

        // Range check the ROUNDED value
        let (overflow, result) = match op {
            AluOp::FCvtWS => {
                if (I32_MIN_F64..I32_MAX_P1_F64).contains(&rounded) {
                    (false, rounded as i32 as i64 as u64)
                } else {
                    (true, if rounded > 0.0 { i32::MAX } else { i32::MIN } as i64 as u64)
                }
            }
            AluOp::FCvtWUS => {
                if (0.0..U32_MAX_P1_F64).contains(&rounded) {
                    (false, rounded as u32 as i32 as i64 as u64)
                } else {
                    (true, if rounded > 0.0 { u32::MAX as i32 as i64 as u64 } else { 0 })
                }
            }
            AluOp::FCvtLS => {
                if (I64_MIN_F64..I64_MAX_P1_F64).contains(&rounded) {
                    (false, rounded as i64 as u64)
                } else {
                    (true, if rounded > 0.0 { i64::MAX } else { i64::MIN } as u64)
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
            execute_f32(
                op,
                std::hint::black_box(a),
                std::hint::black_box(b),
                std::hint::black_box(c),
            )
        } else {
            execute_f64(
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
            let fixed = rmm_fixup(op, a, b, is32, result);
            return (fixed, flags);
        }
        return (result, flags);
    }

    // Non-rm-sensitive ops (comparisons, min/max, sign injection, classify,
    // moves) — delegate to execute_full for manual flag computation.
    // FCvt conversions between FP formats are handled directly in the
    // pipeline execute stages so they don't reach this branch.
    execute_full(op, a, b, c, is32)
}

/// Executes a floating-point operation with an explicit rounding mode,
/// discarding accrued exception flags.
///
/// Thin wrapper around [`execute_full_rm`] preserved for existing
/// callers (unit tests) that want only the result value.
pub fn execute_with_rm(op: AluOp, a: u64, b: u64, c: u64, is32: bool, rm: RoundingMode) -> u64 {
    execute_full_rm(op, a, b, c, false, is32, rm).0
}

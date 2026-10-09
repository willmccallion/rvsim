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
pub mod host;

pub mod nan_handling;

pub mod exact;
pub mod half;

use crate::isa::op::AluOp;

use self::nan_handling::unbox_f32;
use crate::isa::fp::{FpFlags, RoundingMode};
use arith::{execute_f32, execute_f64};
use convert::{
    I32_MAX_P1_F64, I32_MIN_F64, I64_MAX_P1_F64, I64_MIN_F64, U32_MAX_P1_F64, U64_MAX_P1_F64,
    round_to_integer,
};
use exact::{Exact, Format};
use half::execute_f16;
use host::{clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode};
use nan_handling::box_f32_canon;
use nan_handling::{is_snan_f32, is_snan_f64};

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

/// The integer type an `fcvt` from floating point produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntTarget {
    I32,
    U32,
    I64,
    U64,
}

impl IntTarget {
    /// The target of `op`, when it converts floating point to an integer.
    const fn of(op: AluOp) -> Option<Self> {
        match op {
            AluOp::FCvtWS => Some(Self::I32),
            AluOp::FCvtWUS => Some(Self::U32),
            AluOp::FCvtLS => Some(Self::I64),
            AluOp::FCvtLUS => Some(Self::U64),
            _ => None,
        }
    }

    /// The largest value, as the sign-extended register image RISC-V
    /// writes for the type.
    const fn max(self) -> u64 {
        match self {
            Self::I32 => i32::MAX as i64 as u64,
            Self::U32 => u32::MAX as i32 as i64 as u64,
            Self::I64 => i64::MAX as u64,
            Self::U64 => u64::MAX,
        }
    }

    /// The smallest value, as the register image.
    const fn min(self) -> u64 {
        match self {
            Self::I32 => i32::MIN as i64 as u64,
            Self::U32 | Self::U64 => 0,
            Self::I64 => i64::MIN as u64,
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
    is_f16: bool,
    is32: bool,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    // Half-precision (Zfh) ops are handled entirely in software.
    if is_f16 {
        return execute_f16(op, a, b, c, rm);
    }
    // Float-to-integer conversions need rounding-mode-aware handling.
    if let Some(target) = IntTarget::of(op) {
        let val = if is32 { unbox_f32(a) as f64 } else { f64::from_bits(a) };
        let mut flags = FpFlags::NONE;

        if val.is_nan() {
            // NaN → positive max for the target type
            flags = flags | FpFlags::NV;
            return (target.max(), flags);
        }

        if val.is_infinite() {
            flags = flags | FpFlags::NV;
            let result = if val > 0.0 { target.max() } else { target.min() };
            return (result, flags);
        }

        // Round to integer using the specified rounding mode
        let rounded = round_to_integer(val, rm);
        let inexact = val != rounded;

        // Range check the ROUNDED value
        let (overflow, result) = match target {
            IntTarget::I32 => {
                if (I32_MIN_F64..I32_MAX_P1_F64).contains(&rounded) {
                    (false, rounded as i32 as i64 as u64)
                } else {
                    (true, if rounded > 0.0 { target.max() } else { target.min() })
                }
            }
            IntTarget::U32 => {
                if (0.0..U32_MAX_P1_F64).contains(&rounded) {
                    (false, rounded as u32 as i32 as i64 as u64)
                } else {
                    (true, if rounded > 0.0 { target.max() } else { target.min() })
                }
            }
            IntTarget::I64 => {
                if (I64_MIN_F64..I64_MAX_P1_F64).contains(&rounded) {
                    (false, rounded as i64 as u64)
                } else {
                    (true, if rounded > 0.0 { target.max() } else { target.min() })
                }
            }
            IntTarget::U64 => {
                if rounded < 0.0 {
                    (true, target.min())
                } else if rounded >= U64_MAX_P1_F64 {
                    (true, target.max())
                } else {
                    (false, rounded as u64)
                }
            }
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
    // four hardware modes (RNE/RTZ/RDN/RUP); RMM runs as RNE and is rounded
    // again in software below when inexact. `black_box` prevents the
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

        let format = if is32 { Format::Single } else { Format::Double };
        let operand = |value: u64| {
            if is32 {
                Exact::of_f32(unbox_f32(value))
            } else {
                Exact::of_f64(f64::from_bits(value))
            }
        };
        let exact = || exact::of_operation(op, operand(a), operand(b), operand(c));
        if let Some((bits, flags)) = exact::rmm_correction(rm, flags, format, exact) {
            let boxed = if is32 { box_f32_canon(f32::from_bits(bits as u32)) } else { bits };
            return (boxed, flags);
        }
        return (result, flags);
    }

    // Non-rm-sensitive ops (comparisons, min/max, sign injection, classify,
    // moves) — delegate to execute_full for manual flag computation.
    // FCvt conversions between FP formats are handled directly in the
    // pipeline execute stages so they don't reach this branch.
    execute_full(op, a, b, c, is32)
}

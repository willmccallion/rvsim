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

use crate::isa::op::{FpCompareOp, FpOp};

use self::nan_handling::unbox_f32;
use crate::isa::fp::{FpFlags, FpFormat, RoundingMode};
use arith::{execute_f32, execute_f64};
use convert::fp_to_int_register;
pub use convert::{IntTarget, fp_to_int};
use exact::Exact;
use half::execute_f16;
use host::{clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode};
use nan_handling::box_f32_canon;
use nan_handling::{is_snan_f32, is_snan_f64};

/// Executes a floating-point operation with an explicit rounding mode,
/// returning the result and accrued exception flags.
///
/// This is the primary entry point from the pipeline execute stages.
/// For float-to-integer conversions, the rounding mode determines how
/// the floating-point value is rounded before being cast to integer.
/// For FP arithmetic, the rounding mode affects the result precision.
pub fn execute_full_rm(
    op: FpOp,
    a: u64,
    b: u64,
    c: u64,
    format: FpFormat,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    // Half-precision (Zfh) ops are handled entirely in software.
    let is32 = match format {
        FpFormat::Half => return execute_f16(op, a, b, c, rm),
        FpFormat::Single => true,
        FpFormat::Double => false,
    };
    if let Some(target) = IntTarget::of(op) {
        let value = if is32 { f64::from(unbox_f32(a)) } else { f64::from_bits(a) };
        return fp_to_int_register(value, target, rm);
    }

    // Rounding-mode-sensitive arithmetic: set the host FPU rounding mode,
    // clear exception flags, run the op, read flags, and restore. The host
    // FPU is IEEE 754 compliant so this gives bit-exact results for all
    // four hardware modes (RNE/RTZ/RDN/RUP); RMM runs as RNE and is rounded
    // again in software below when inexact. `black_box` prevents the
    // optimizer from constant-folding FP ops at compile time or reordering
    // them across the feclearexcept/fetestexcept calls.
    let is_rm_sensitive_arith = matches!(op, FpOp::Arith(_) | FpOp::Sqrt | FpOp::Fma(_));

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

    // Conversions between FP formats and from integers are handled in the
    // execute stages, so what is left rounds nothing.
    execute_unrounded(op, a, b, c, is32)
}

/// Executes an operation that rounds nothing: a comparison, min/max, sign
/// injection, classification or move, with the flags RISC-V gives it.
fn execute_unrounded(op: FpOp, a: u64, b: u64, c: u64, is32: bool) -> (u64, FpFlags) {
    let operands = [a, b];
    let is_snan = |bits: u64| {
        if is32 { is_snan_f32(unbox_f32(bits)) } else { is_snan_f64(f64::from_bits(bits)) }
    };
    let is_nan =
        |bits: u64| if is32 { unbox_f32(bits).is_nan() } else { f64::from_bits(bits).is_nan() };
    let is_invalid = match op {
        // FEQ, FMIN and FMAX raise NV only on a signaling NaN.
        FpOp::Compare(FpCompareOp::Eq) | FpOp::Min | FpOp::Max => operands.into_iter().any(is_snan),
        // FLT and FLE raise NV on any NaN.
        FpOp::Compare(FpCompareOp::Lt | FpCompareOp::Le) => operands.into_iter().any(is_nan),
        _ => false,
    };
    let flags = if is_invalid { FpFlags::NV } else { FpFlags::NONE };
    let result = if is32 { execute_f32(op, a, b, c) } else { execute_f64(op, a, b, c) };
    (result, flags)
}

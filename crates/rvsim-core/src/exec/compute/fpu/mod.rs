//! Floating-Point Unit (FPU).
//!
//! The scalar floating-point operations, written once over the [`Float`]
//! formats ([`SoftF16`], f32 and f64), and the helpers the vector unit
//! shares with it.

// IEEE 754 FEQ requires exact bit-pattern comparison — float_cmp is intentional here.
#![allow(clippy::float_cmp)]

mod convert;
pub mod exact;
pub mod float;
pub mod half;
pub mod host;
pub mod nan_handling;

use crate::isa::bits::sign_extend;
use crate::isa::fp::{FpClass, FpFlags, FpFormat, RoundingMode};
use crate::isa::op::{FmaSigns, FpArithOp, FpCompareOp, FpOp, SignInjectOp};
use convert::fp_to_int_register;
pub use convert::{IntTarget, fp_to_int};
use float::{Float, SoftF16};
use host::{restore_host_round_mode, set_host_round_mode};

/// Executes the floating-point `op` in `format` under `rm` on the register
/// images `[rs1, rs2, rs3]`, `rs1` an integer register's value for a
/// conversion or move from one. Returns the destination register's value
/// and the flags raised.
pub fn execute(op: FpOp, operands: [u64; 3], format: FpFormat, rm: RoundingMode) -> (u64, FpFlags) {
    let saved = rounds_on_host(op).then(|| set_host_round_mode(rm));
    let result = match format {
        FpFormat::Half => execute_in::<SoftF16>(op, operands, rm),
        FpFormat::Single => execute_in::<f32>(op, operands, rm),
        FpFormat::Double => execute_in::<f64>(op, operands, rm),
    };
    if let Some(saved) = saved {
        restore_host_round_mode(saved);
    }
    result
}

/// Whether `op` rounds on the host FPU, which must then round as the
/// instruction's rounding mode says.
const fn rounds_on_host(op: FpOp) -> bool {
    matches!(
        op,
        FpOp::Arith(_) | FpOp::Sqrt | FpOp::Fma(_) | FpOp::FromInt(..) | FpOp::FromFloat(_)
    )
}

/// [`execute`] in the format `F`.
fn execute_in<F: Float>(op: FpOp, [a, b, c]: [u64; 3], rm: RoundingMode) -> (u64, FpFlags) {
    let (x, y, z) = (F::from_register(a), F::from_register(b), F::from_register(c));
    let rounded = |(result, flags): (F, FpFlags)| (result.canonical().to_register(), flags);
    match op {
        FpOp::Arith(op) => rounded(arith(op, x, y, rm)),
        FpOp::Sqrt => rounded(x.sqrt(rm)),
        FpOp::Fma(signs) => {
            let (product_a, product_b, addend) = fma_terms(signs, x, y, z);
            rounded(product_a.mul_add(product_b, addend, rm))
        }
        FpOp::Min => (x.min(y).to_register(), invalid_if_signaling(x, y)),
        FpOp::Max => (x.max(y).to_register(), invalid_if_signaling(x, y)),
        FpOp::SignInject(op) => (sign_inject(op, x, y).to_register(), FpFlags::NONE),
        FpOp::Compare(op) => {
            let (holds, flags) = compare(op, x, y);
            (u64::from(holds), flags)
        }
        FpOp::Class => (FpClass::of(F::FORMAT, x.raw_bits()).mask(), FpFlags::NONE),
        // fmv.x.*: the register's low bits as they are, NaN-boxed or not.
        FpOp::MoveToInt => (sign_extend(a, F::BITS) as u64, FpFlags::NONE),
        FpOp::MoveFromInt => (F::from_raw_bits(a).to_register(), FpFlags::NONE),
        FpOp::ToInt(width, signedness) => {
            fp_to_int_register(x.to_f64(), IntTarget::of_width(width, signedness), rm)
        }
        FpOp::FromInt(width, signedness) => rounded(F::from_int(a, (width, signedness), rm)),
        FpOp::FromFloat(source) => rounded(convert_from::<F>(source, a, rm)),
    }
}

/// `x op y`.
fn arith<F: Float>(op: FpArithOp, x: F, y: F, rm: RoundingMode) -> (F, FpFlags) {
    match op {
        FpArithOp::Add => x.add(y, rm),
        FpArithOp::Sub => x.sub(y, rm),
        FpArithOp::Mul => x.mul(y, rm),
        FpArithOp::Div => x.div(y, rm),
    }
}

/// The multiplicands and addend of `±(a × b) ± c` with the `signs`.
pub fn fma_terms<T: std::ops::Neg<Output = T>>(signs: FmaSigns, a: T, b: T, c: T) -> (T, T, T) {
    match signs {
        FmaSigns::Macc => (a, b, c),
        FmaSigns::NMacc => (-a, b, -c),
        FmaSigns::MSac => (a, b, -c),
        FmaSigns::NMSac => (-a, b, c),
    }
}

/// NV when either operand is a signaling NaN, as minNum and maxNum raise.
pub fn invalid_if_signaling<F: Float>(a: F, b: F) -> FpFlags {
    if a.is_signaling_nan() || b.is_signaling_nan() { FpFlags::NV } else { FpFlags::NONE }
}

/// The magnitude of `x` with the sign `op` takes from `y`.
pub fn sign_inject<F: Float>(op: SignInjectOp, x: F, y: F) -> F {
    let (x_bits, y_bits) = (x.raw_bits(), y.raw_bits());
    let magnitude = x_bits & !F::SIGN_BIT;
    F::from_raw_bits(match op {
        SignInjectOp::Copy => magnitude | (y_bits & F::SIGN_BIT),
        SignInjectOp::Negate => magnitude | (!y_bits & F::SIGN_BIT),
        SignInjectOp::Xor => x_bits ^ (y_bits & F::SIGN_BIT),
    })
}

/// Whether `a op b` holds, with the flags: equality raises NV only on a
/// signaling NaN, the orderings on any NaN.
pub fn compare<F: Float>(op: FpCompareOp, a: F, b: F) -> (bool, FpFlags) {
    let (x, y) = (a.to_f64(), b.to_f64());
    let is_invalid = match op {
        FpCompareOp::Eq | FpCompareOp::Ne => a.is_signaling_nan() || b.is_signaling_nan(),
        FpCompareOp::Lt | FpCompareOp::Le | FpCompareOp::Gt | FpCompareOp::Ge => {
            a.is_nan() || b.is_nan()
        }
    };
    let holds = match op {
        FpCompareOp::Eq => x == y,
        FpCompareOp::Ne => x != y,
        FpCompareOp::Lt => x < y,
        FpCompareOp::Le => x <= y,
        FpCompareOp::Gt => x > y,
        FpCompareOp::Ge => x >= y,
    };
    (holds, if is_invalid { FpFlags::NV } else { FpFlags::NONE })
}

/// `fcvt` of the `source`-format value in `register` to `F`. Widening is
/// exact; the host raises NV for a signaling NaN only when it narrows, so
/// NV is raised here for every format.
fn convert_from<F: Float>(source: FpFormat, register: u64, rm: RoundingMode) -> (F, FpFlags) {
    let (value, is_signaling) = match source {
        FpFormat::Half => widened::<SoftF16>(register),
        FpFormat::Single => widened::<f32>(register),
        FpFormat::Double => widened::<f64>(register),
    };
    let (result, flags) = F::from_f64_rounded(value, rm);
    (result, if is_signaling { flags | FpFlags::NV } else { flags })
}

/// The value of the `S` register image, as an f64, and whether it is a
/// signaling NaN.
fn widened<S: Float>(register: u64) -> (f64, bool) {
    let value = S::from_register(register);
    (value.to_f64(), value.is_signaling_nan())
}

//! Zfh arithmetic: half-precision results and flags, which the FPU computes
//! through f64 and rounds to f16 in software.

use crate::exec::compute::fpu;
use crate::exec::compute::fpu::float::{Float, SoftF16};
use crate::isa::fp::{FpFlags, FpFormat, RoundingMode};
use crate::isa::op::{FmaSigns, FpArithOp, FpOp};

const ZERO: u16 = 0x0000;
const ONE: u16 = 0x3C00;
const TWO: u16 = 0x4000;
const MINUS_ONE: u16 = 0xBC00;
const INFINITY: u16 = 0x7C00;
const CANONICAL_NAN: u16 = 0x7E00;
const SIGNALING_NAN: u16 = 0x7D00;
/// The smallest subnormal, 2^-24.
const SMALLEST: u16 = 0x0001;
/// The f16 just above one.
const ONE_UP: u16 = 0x3C01;

/// `op` on half-precision operands, as the f16 result and its flags.
fn half(op: FpOp, operands: [u16; 3], rm: RoundingMode) -> (u16, FpFlags) {
    let registers = operands.map(|bits| SoftF16::from_raw_bits(bits.into()).to_register());
    let (result, flags) = fpu::execute(op, registers, FpFormat::Half, rm);
    (result as u16, flags)
}

fn divide(a: u16, b: u16) -> (u16, FpFlags) {
    half(FpOp::Arith(FpArithOp::Div), [a, b, ZERO], RoundingMode::Rne)
}

#[test]
fn one_over_zero_is_infinity_raising_only_dz() {
    assert_eq!(divide(ONE, ZERO), (INFINITY, FpFlags::DZ));
}

#[test]
fn zero_over_zero_is_the_canonical_nan_raising_nv() {
    assert_eq!(divide(ZERO, ZERO), (CANONICAL_NAN, FpFlags::NV));
}

#[test]
fn dividing_by_a_signaling_nan_raises_nv() {
    assert_eq!(divide(ONE, SIGNALING_NAN), (CANONICAL_NAN, FpFlags::NV));
}

#[test]
fn the_square_root_of_a_negative_is_the_canonical_nan_raising_nv() {
    let root = half(FpOp::Sqrt, [MINUS_ONE, ZERO, ZERO], RoundingMode::Rne);

    assert_eq!(root, (CANONICAL_NAN, FpFlags::NV));
}

#[test]
fn min_and_max_return_the_number_when_one_operand_is_a_quiet_nan() {
    let min_max = |op, a, b| half(op, [a, b, ZERO], RoundingMode::Rne).0;

    assert_eq!(min_max(FpOp::Min, CANONICAL_NAN, TWO), TWO);
    assert_eq!(min_max(FpOp::Max, ONE, TWO), TWO);
    assert_eq!(min_max(FpOp::Min, CANONICAL_NAN, CANONICAL_NAN), CANONICAL_NAN);
}

#[test]
fn a_fused_multiply_add_rounds_once_from_the_exact_value() {
    // 2^-24 × 2^-24 + 1 is too wide for f64, so only rounding the exact
    // sum up reaches the f16 above one.
    let fmadd = |rm| half(FpOp::Fma(FmaSigns::Macc), [SMALLEST, SMALLEST, ONE], rm);

    assert_eq!(fmadd(RoundingMode::Rup), (ONE_UP, FpFlags::NX));
    assert_eq!(fmadd(RoundingMode::Rne), (ONE, FpFlags::NX));
}

//! RMM (round to nearest, ties to max magnitude) from software rounding: a
//! tie goes away from zero, everything else as round-to-nearest-even.

use std::hint::black_box;

use proptest::prelude::*;

use crate::exec::compute::fpu::exact::{self, Exact, Format};
use crate::exec::compute::fpu::host::{restore_host_round_mode, set_host_round_mode};
use crate::isa::fp::{FpFlags, RoundingMode};

fn single(value: f32) -> Exact {
    Exact::of(u64::from(value.to_bits()), Format::Single)
}

fn double(value: f64) -> Exact {
    Exact::of(value.to_bits(), Format::Double)
}

fn rmm_single(exact: Exact) -> f32 {
    f32::from_bits(exact::round(exact, Format::Single, RoundingMode::Rmm).0 as u32)
}

fn rmm_double(exact: Exact) -> f64 {
    f64::from_bits(exact::round(exact, Format::Double, RoundingMode::Rmm).0)
}

fn rne_double(exact: Exact) -> f64 {
    f64::from_bits(exact::round(exact, Format::Double, RoundingMode::Rne).0)
}

#[test]
fn a_single_precision_tie_rounds_away_from_zero() {
    let half_ulp = 2f32.powi(-24);

    assert_eq!(rmm_single(exact::add(single(1.0), single(half_ulp))).to_bits(), 0x3f80_0001);
    assert_eq!(rmm_single(exact::sub(single(-1.0), single(half_ulp))).to_bits(), 0xbf80_0001);
}

#[test]
fn a_double_precision_product_tie_rounds_away_where_nearest_even_rounds_down() {
    let ulp = 2f64.powi(-52);
    let tie = exact::mul(double(1.5), double(3.0f64.mul_add(ulp, 1.0)));

    assert_eq!(rne_double(tie), 4.0f64.mul_add(ulp, 1.5));
    assert_eq!(rmm_double(tie), 5.0f64.mul_add(ulp, 1.5));
}

#[test]
fn a_fused_tie_rounds_away_from_zero() {
    let tie = exact::mul_add(double(1.0), double(1.0), double(2f64.powi(-53)));

    assert_eq!(rmm_double(tie), 1.0 + 2f64.powi(-52));
}

#[test]
fn an_integer_halfway_between_floats_rounds_away_from_zero() {
    let tie = Exact::of_integer((1 << 24) + 1);

    assert_eq!(rmm_single(tie), 16_777_218.0);
}

#[test]
fn narrowing_a_tie_rounds_away_from_zero() {
    let tie = double(1.0 + 2f64.powi(-24));

    assert_eq!(rmm_single(tie).to_bits(), 0x3f80_0001);
}

#[test]
fn half_the_smallest_subnormal_rounds_up_to_it_and_underflows() {
    let tie = exact::mul(single(2f32.powi(-75)), single(2f32.powi(-75)));

    let (bits, flags) = exact::round(tie, Format::Single, RoundingMode::Rmm);

    assert_eq!(bits, 0x0000_0001);
    assert_eq!(flags.bits(), FpFlags::UF.union(FpFlags::NX).bits());
}

#[test]
fn half_an_ulp_past_the_largest_finite_number_overflows() {
    let tie = exact::add(single(f32::MAX), single(2f32.powi(103)));

    let (bits, flags) = exact::round(tie, Format::Single, RoundingMode::Rmm);

    assert_eq!(bits, u64::from(f32::INFINITY.to_bits()));
    assert_eq!(flags.bits(), FpFlags::OF.union(FpFlags::NX).bits());
}

/// The single-precision value nearest `exact`, ties away from zero, from
/// the host's round-toward-zero and the midpoint above it.
fn nearest_ties_away(exact: f64) -> f32 {
    let saved = set_host_round_mode(RoundingMode::Rtz);
    let toward_zero = black_box(black_box(exact) as f32);
    restore_host_round_mode(saved);
    if f64::from(toward_zero) == exact {
        return toward_zero;
    }
    let away = f32::from_bits(toward_zero.to_bits() + 1);
    let midpoint = f64::midpoint(f64::from(toward_zero), f64::from(away));
    if exact.abs() >= midpoint.abs() { away } else { toward_zero }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4096, ..ProptestConfig::default() })]

    #[test]
    fn a_single_precision_product_rounds_to_nearest_ties_away(a in any::<u32>(), b in any::<u32>()) {
        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
        prop_assume!(x.is_finite() && y.is_finite() && x != 0.0 && y != 0.0);
        let exact_product = f64::from(x) * f64::from(y);
        prop_assume!(exact_product.abs() <= f64::from(f32::MAX));

        let rounded = rmm_single(exact::mul(single(x), single(y)));

        prop_assert_eq!(rounded.to_bits(), nearest_ties_away(exact_product).to_bits());
    }

    #[test]
    fn division_and_square_root_never_tie(a in any::<u64>(), b in any::<u64>()) {
        let (x, y) = (f64::from_bits(a), f64::from_bits(b));
        prop_assume!(x.is_finite() && y.is_finite() && x != 0.0 && y != 0.0);
        let quotient = exact::div(double(x), double(y));
        let root = exact::sqrt(double(x.abs()));

        prop_assert_eq!(rmm_double(quotient).to_bits(), rne_double(quotient).to_bits());
        prop_assert_eq!(rmm_double(root).to_bits(), rne_double(root).to_bits());
    }
}

//! Software rounding agrees with the host FPU in the four rounding modes the
//! host has, for every operation and format, so it can be trusted for RMM,
//! which the host lacks. Half precision, which the host lacks entirely,
//! computes through f64 and rounds once more; it agrees with rounding the
//! exact result in all five modes.

use std::hint::black_box;

use proptest::prelude::*;

use crate::exec::compute::fpu::exact::{self, Exact};
use crate::exec::compute::fpu::float::{Float, SoftF16};
use crate::exec::compute::fpu::host::{
    clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode,
};
use crate::isa::fp::{FpFlags, FpFormat, RoundingMode};

const HOST_MODES: [RoundingMode; 4] =
    [RoundingMode::Rne, RoundingMode::Rtz, RoundingMode::Rdn, RoundingMode::Rup];
const ALL_MODES: [RoundingMode; 5] =
    [RoundingMode::Rne, RoundingMode::Rtz, RoundingMode::Rdn, RoundingMode::Rup, RoundingMode::Rmm];
const ROUNDING_FLAGS: u8 = FpFlags::NX.bits() | FpFlags::UF.bits() | FpFlags::OF.bits();

/// Runs `op` on the host FPU in `rm` and returns its result and flags.
fn on_host<T>(rm: RoundingMode, op: impl FnOnce() -> T) -> (T, FpFlags) {
    let saved = set_host_round_mode(rm);
    clear_host_fp_flags();
    let result = black_box(op());
    let flags = read_host_fp_flags();
    restore_host_round_mode(saved);
    (result, flags)
}

/// Bit patterns weighted toward the hard cases: subnormals, the smallest
/// normals, the largest finite values and short significands.
fn f32_bits() -> impl Strategy<Value = u32> {
    prop_oneof![
        any::<u32>(),
        (any::<bool>(), 0u32..0x0100_0000).prop_map(|(neg, low)| (u32::from(neg) << 31) | low),
        (any::<bool>(), 0x7e00_0000u32..0x7f80_0000)
            .prop_map(|(neg, x)| (u32::from(neg) << 31) | x),
        (any::<u32>(), 0u32..8).prop_map(|(x, keep)| x & !((1 << (20 + keep)) - 1)),
    ]
    .prop_filter("finite", |bits| f32::from_bits(*bits).is_finite())
}

fn f64_bits() -> impl Strategy<Value = u64> {
    prop_oneof![
        any::<u64>(),
        (any::<bool>(), 0u64..0x0020_0000_0000_0000)
            .prop_map(|(neg, low)| (u64::from(neg) << 63) | low),
        (any::<bool>(), 0x7fc0_0000_0000_0000u64..0x7ff0_0000_0000_0000)
            .prop_map(|(neg, x)| (u64::from(neg) << 63) | x),
        (any::<u64>(), 0u32..8).prop_map(|(x, keep)| x & !((1u64 << (45 + keep)) - 1)),
    ]
    .prop_filter("finite", |bits| f64::from_bits(*bits).is_finite())
}

/// Half-precision bit patterns, weighted toward subnormals, the smallest
/// normals and the largest finite values.
fn f16_bits() -> impl Strategy<Value = u16> {
    prop_oneof![
        any::<u16>(),
        (any::<bool>(), 0u16..0x0800).prop_map(|(neg, low)| (u16::from(neg) << 15) | low),
        (any::<bool>(), 0x7800u16..0x7c00).prop_map(|(neg, x)| (u16::from(neg) << 15) | x),
    ]
    .prop_filter("finite", |bits| bits & 0x7c00 != 0x7c00)
}

/// f64 bit patterns whose exponents span f16's range and a little beyond,
/// so that rounding them to f16 is rarely exact or trivial.
fn half_range_f64_bits() -> impl Strategy<Value = u64> {
    const F64_BIAS: i64 = 1023;
    (any::<bool>(), -27i64..17, any::<u64>()).prop_map(|(neg, exponent, fraction)| {
        (u64::from(neg) << 63) | ((exponent + F64_BIAS).cast_unsigned() << 52) | (fraction >> 12)
    })
}

/// Runs `op` with the host FPU in `rm`, as [`SoftF16`]'s f64 step needs.
fn in_mode<T>(rm: RoundingMode, op: impl FnOnce() -> T) -> T {
    let saved = set_host_round_mode(rm);
    let result = black_box(op());
    restore_host_round_mode(saved);
    result
}

/// Checks software rounding of `exact` against the host's `host_bits` and
/// `host_flags`, where the host result is finite and came from rounding.
fn agrees(
    exact: Exact,
    format: FpFormat,
    rm: RoundingMode,
    host_bits: u64,
    host_flags: FpFlags,
) -> Result<(), TestCaseError> {
    let (bits, flags) = exact::round(exact, format, rm);
    prop_assert_eq!(bits, host_bits, "{:?} {:?} of {:?}", format, rm, exact);
    prop_assert_eq!(
        flags.bits() & ROUNDING_FLAGS,
        host_flags.bits() & ROUNDING_FLAGS,
        "{:?} {:?} of {:?}",
        format,
        rm,
        exact
    );
    Ok(())
}

/// Whether the host result is one rounding produced: finite operands gave
/// a finite or overflowed result without an invalid or divide-by-zero, and
/// not an exact zero, whose sign is the operation's business.
fn rounded(host_bits: u64, host_flags: FpFlags, sign_bit: u64) -> bool {
    !host_flags.contains(FpFlags::NV)
        && !host_flags.contains(FpFlags::DZ)
        && (host_bits & !sign_bit != 0 || host_flags.contains(FpFlags::NX))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4096, ..ProptestConfig::default() })]

    #[test]
    fn single_precision_arithmetic_rounds_as_the_host_does(
        a in f32_bits(), b in f32_bits(), c in f32_bits(),
    ) {
        let (x, y, z) = (f32::from_bits(a), f32::from_bits(b), f32::from_bits(c));
        let (ea, eb, ec) = (
            Exact::of(u64::from(a), FpFormat::Single),
            Exact::of(u64::from(b), FpFormat::Single),
            Exact::of(u64::from(c), FpFormat::Single),
        );
        let sign = 1u64 << 31;
        for rm in HOST_MODES {
            let cases: [(f32, FpFlags, Option<Exact>); 6] = [
                with_exact(on_host(rm, || black_box(x) + black_box(y)), Some(exact::add(ea, eb))),
                with_exact(on_host(rm, || black_box(x) - black_box(y)), Some(exact::sub(ea, eb))),
                with_exact(on_host(rm, || black_box(x) * black_box(y)), Some(exact::mul(ea, eb))),
                with_exact(
                    on_host(rm, || black_box(x) / black_box(y)),
                    (a & !(1 << 31) != 0 && b & !(1 << 31) != 0).then(|| exact::div(ea, eb)),
                ),
                with_exact(
                    on_host(rm, || black_box(x.abs()).sqrt()),
                    (a & !(1 << 31) != 0)
                        .then(|| exact::sqrt(Exact { negative: false, ..ea })),
                ),
                with_exact(
                    on_host(rm, || black_box(x).mul_add(black_box(y), black_box(z))),
                    Some(exact::mul_add(ea, eb, ec)),
                ),
            ];
            for (host, flags, exact) in cases {
                let host_bits = u64::from(host.to_bits());
                if let Some(exact) = exact.filter(|_| rounded(host_bits, flags, sign)) {
                    agrees(exact, FpFormat::Single, rm, host_bits, flags)?;
                }
            }
        }
    }

    #[test]
    fn double_precision_arithmetic_rounds_as_the_host_does(
        a in f64_bits(), b in f64_bits(), c in f64_bits(),
    ) {
        let (x, y, z) = (f64::from_bits(a), f64::from_bits(b), f64::from_bits(c));
        let (ea, eb, ec) =
            (Exact::of(a, FpFormat::Double), Exact::of(b, FpFormat::Double), Exact::of(c, FpFormat::Double));
        let sign = 1u64 << 63;
        for rm in HOST_MODES {
            let cases: [(f64, FpFlags, Option<Exact>); 6] = [
                with_exact(on_host(rm, || black_box(x) + black_box(y)), Some(exact::add(ea, eb))),
                with_exact(on_host(rm, || black_box(x) - black_box(y)), Some(exact::sub(ea, eb))),
                with_exact(on_host(rm, || black_box(x) * black_box(y)), Some(exact::mul(ea, eb))),
                with_exact(
                    on_host(rm, || black_box(x) / black_box(y)),
                    (a & !sign != 0 && b & !sign != 0).then(|| exact::div(ea, eb)),
                ),
                with_exact(
                    on_host(rm, || black_box(x.abs()).sqrt()),
                    (a & !sign != 0).then(|| exact::sqrt(Exact { negative: false, ..ea })),
                ),
                with_exact(
                    on_host(rm, || black_box(x).mul_add(black_box(y), black_box(z))),
                    Some(exact::mul_add(ea, eb, ec)),
                ),
            ];
            for (host, flags, exact) in cases {
                let host_bits = host.to_bits();
                if let Some(exact) = exact.filter(|_| rounded(host_bits, flags, sign)) {
                    agrees(exact, FpFormat::Double, rm, host_bits, flags)?;
                }
            }
        }
    }

    #[test]
    fn half_precision_arithmetic_rounds_as_the_exact_result_does(
        a in f16_bits(), b in f16_bits(), c in f16_bits(),
    ) {
        let half = |bits: u16| SoftF16::from_raw_bits(u64::from(bits));
        let (x, y, z) = (half(a), half(b), half(c));
        let (ea, eb, ec) = (
            Exact::of(u64::from(a), FpFormat::Half),
            Exact::of(u64::from(b), FpFormat::Half),
            Exact::of(u64::from(c), FpFormat::Half),
        );
        let sign = 1u16 << 15;
        let nonzero = |bits: u16| bits & !sign != 0;
        for rm in ALL_MODES {
            let cases: [((SoftF16, FpFlags), Option<Exact>); 6] = [
                (in_mode(rm, || x.add(y, rm)), Some(exact::add(ea, eb))),
                (in_mode(rm, || x.sub(y, rm)), Some(exact::sub(ea, eb))),
                (in_mode(rm, || x.mul(y, rm)), Some(exact::mul(ea, eb))),
                (
                    in_mode(rm, || x.div(y, rm)),
                    (nonzero(a) && nonzero(b)).then(|| exact::div(ea, eb)),
                ),
                (
                    in_mode(rm, || half(a & !sign).sqrt(rm)),
                    nonzero(a).then(|| exact::sqrt(Exact { negative: false, ..ea })),
                ),
                (in_mode(rm, || x.mul_add(y, z, rm)), Some(exact::mul_add(ea, eb, ec))),
            ];
            for ((result, flags), exact) in cases {
                let bits = result.raw_bits();
                if let Some(exact) = exact.filter(|_| rounded(bits, flags, u64::from(sign))) {
                    agrees(exact, FpFormat::Half, rm, bits, flags)?;
                }
            }
        }
    }

    #[test]
    fn conversions_to_half_round_as_the_exact_value_does(wide in half_range_f64_bits()) {
        for rm in ALL_MODES {
            let (half, flags) = SoftF16::from_f64_rounded(f64::from_bits(wide), rm);
            let bits = half.raw_bits();
            if rounded(bits, flags, 1 << 15) {
                agrees(Exact::of(wide, FpFormat::Double), FpFormat::Half, rm, bits, flags)?;
            }
        }
    }

    #[test]
    fn conversions_round_as_the_host_does(int in any::<i64>(), uint in any::<u64>(), wide in f64_bits()) {
        for rm in HOST_MODES {
            let (h, f) = on_host(rm, || black_box(int) as f32);
            agrees(Exact::of_integer(i128::from(int)), FpFormat::Single, rm, u64::from(h.to_bits()), f)?;
            let (h, f) = on_host(rm, || black_box(int) as f64);
            agrees(Exact::of_integer(i128::from(int)), FpFormat::Double, rm, h.to_bits(), f)?;
            let (h, f) = on_host(rm, || black_box(uint) as f32);
            agrees(Exact::of_unsigned(u128::from(uint)), FpFormat::Single, rm, u64::from(h.to_bits()), f)?;
            let (h, f) = on_host(rm, || black_box(uint) as f64);
            agrees(Exact::of_unsigned(u128::from(uint)), FpFormat::Double, rm, h.to_bits(), f)?;
            let (h, f) = on_host(rm, || black_box(f64::from_bits(wide)) as f32);
            if rounded(u64::from(h.to_bits()), f, 1 << 31) {
                agrees(Exact::of(wide, FpFormat::Double), FpFormat::Single, rm, u64::from(h.to_bits()), f)?;
            }
        }
    }
}

fn with_exact<T>((host, flags): (T, FpFlags), exact: Option<Exact>) -> (T, FpFlags, Option<Exact>) {
    (host, flags, exact)
}

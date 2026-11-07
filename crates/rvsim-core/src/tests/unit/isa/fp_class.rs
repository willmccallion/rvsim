//! `FpClass`: the class `FCLASS` reports for each format's encodings.

use crate::isa::fp::{FpClass, FpFormat};

/// One encoding of every class, in `FpClass` order, for a format whose
/// exponent and fraction are `exp_bits` and `frac_bits` wide.
fn one_of_each(exp_bits: u32, frac_bits: u32) -> [u64; 10] {
    let sign = 1u64 << (exp_bits + frac_bits);
    let exp_max = ((1u64 << exp_bits) - 1) << frac_bits;
    let quiet = 1u64 << (frac_bits - 1);
    let one = 1u64 << frac_bits;
    [sign | exp_max, sign | one, sign | 1, sign, 0, 1, one, exp_max, exp_max | 1, exp_max | quiet]
}

const CLASSES: [FpClass; 10] = [
    FpClass::NegInfinity,
    FpClass::NegNormal,
    FpClass::NegSubnormal,
    FpClass::NegZero,
    FpClass::PosZero,
    FpClass::PosSubnormal,
    FpClass::PosNormal,
    FpClass::PosInfinity,
    FpClass::SignalingNan,
    FpClass::QuietNan,
];

#[test]
fn each_format_classifies_one_encoding_of_every_class() {
    for (format, exp_bits, frac_bits) in
        [(FpFormat::Half, 5, 10), (FpFormat::Single, 8, 23), (FpFormat::Double, 11, 52)]
    {
        let classes = one_of_each(exp_bits, frac_bits).map(|bits| FpClass::of(format, bits));

        assert_eq!(classes, CLASSES, "{format:?}");
    }
}

#[test]
fn the_mask_sets_the_bit_fclass_numbers_the_class() {
    let masks = CLASSES.map(FpClass::mask);

    assert_eq!(masks, std::array::from_fn(|bit| 1 << bit));
}

#[test]
fn bits_above_the_format_are_ignored() {
    let boxed_negative_zero = 0xFFFF_FFFF_8000_0000;

    assert_eq!(FpClass::of(FpFormat::Single, boxed_negative_zero), FpClass::NegZero);
}

//! Sign and zero extension in `isa::bits`.

use crate::isa::bits::{sext_word, sign_extend, zext_word};

#[test]
fn sign_extend_copies_the_top_kept_bit_upward() {
    assert_eq!(sign_extend(0b10_0000, 6), -32);
    assert_eq!(sign_extend(0b01_1111, 6), 31);
}

#[test]
fn sign_extend_ignores_bits_above_the_width() {
    assert_eq!(sign_extend(0xFFFF_FF00_0000_0001, 12), 1);
}

#[test]
fn sign_extend_of_the_full_width_is_the_value() {
    assert_eq!(sign_extend(u64::MAX, 64), -1);
}

#[test]
fn sext_word_extends_bit_31() {
    assert_eq!(sext_word(0x8000_0000), 0xFFFF_FFFF_8000_0000);
    assert_eq!(sext_word(0x7FFF_FFFF), 0x7FFF_FFFF);
}

#[test]
fn zext_word_clears_the_upper_half() {
    assert_eq!(zext_word(0xFFFF_FFFF_8000_0000), 0x8000_0000);
}

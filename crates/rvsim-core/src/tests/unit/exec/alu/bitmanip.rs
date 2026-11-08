//! B-extension compute: the byte operations, carry-less multiplies,
//! crossbar permutes and packing that have no other unit tests.

use crate::exec::compute::alu::bitmanip::execute;
use crate::isa::op::{BitOp, ClmulOp, IntWidth, MinMaxOp, PackOp};

fn double(op: BitOp, a: u64, b: u64) -> u64 {
    execute(op, a, b, IntWidth::Double)
}

#[test]
fn brev8_reverses_the_bits_of_each_byte_in_place() {
    assert_eq!(double(BitOp::Brev8, 0x0102_0408_1020_4080, 0), 0x8040_2010_0804_0201);
}

#[test]
fn orc_b_sets_every_bit_of_each_nonzero_byte() {
    assert_eq!(double(BitOp::OrcB, 0x0010_0000_8000_0001, 0), 0x00FF_0000_FF00_00FF);
}

#[test]
fn clmul_of_powers_of_two_shifts() {
    assert_eq!(double(BitOp::Clmul(ClmulOp::Clmul), 1 << 40, 1 << 30), 0);
    assert_eq!(double(BitOp::Clmul(ClmulOp::Clmulh), 1 << 40, 1 << 30), 1 << 6);
}

#[test]
fn clmul_xors_partial_products() {
    // (x + 1)(x + 1) = x² + 1 over GF(2).
    assert_eq!(double(BitOp::Clmul(ClmulOp::Clmul), 0b11, 0b11), 0b101);
}

#[test]
fn clmulr_is_bits_126_to_63_of_the_product() {
    let (a, b) = (0x8000_0000_0000_0001, 0x8000_0000_0000_0003);
    let product = (u128::from(double(BitOp::Clmul(ClmulOp::Clmulh), a, b)) << 64)
        | u128::from(double(BitOp::Clmul(ClmulOp::Clmul), a, b));

    assert_eq!(double(BitOp::Clmul(ClmulOp::Clmulr), a, b), (product >> 63) as u64);
}

#[test]
fn xperm4_selects_nibbles_by_index() {
    let table = 0xFEDC_BA98_7654_3210;

    assert_eq!(double(BitOp::Xperm4, table, 0x0123_4567_89AB_CDEF), 0x0123_4567_89AB_CDEF);
}

#[test]
fn xperm8_gives_zero_for_an_index_past_the_last_byte() {
    let table = 0x8877_6655_4433_2211;

    assert_eq!(double(BitOp::Xperm8, table, 0x0000_0000_0000_FF07), 0x1111_1111_1111_0088);
}

#[test]
fn pack_joins_low_halves() {
    assert_eq!(
        double(BitOp::Pack(PackOp::Pack), 0xAAAA_AAAA_1111_2222, 0xBBBB_BBBB_3333_4444),
        0x3333_4444_1111_2222
    );
    assert_eq!(
        execute(BitOp::Pack(PackOp::Packw), 0x1234, 0x8765, IntWidth::Word),
        0xFFFF_FFFF_8765_1234
    );
    assert_eq!(double(BitOp::Pack(PackOp::Packh), 0x1234, 0x5678), 0x7834);
}

#[test]
fn word_min_max_compare_the_sign_extended_low_words() {
    let (negative, positive) = (0xFFFF_FFFF, 1);

    assert_eq!(execute(BitOp::MinMax(MinMaxOp::Min), negative, positive, IntWidth::Word), u64::MAX);
    assert_eq!(
        execute(BitOp::MinMax(MinMaxOp::Maxu), negative, positive, IntWidth::Word),
        u64::MAX
    );
    assert_eq!(execute(BitOp::MinMax(MinMaxOp::Minu), negative, positive, IntWidth::Word), 1);
}

//! B-extension compute: each operation against the Bitmanip spec's
//! definition, at both widths where it has two.

use crate::exec::compute::alu::bitmanip::execute;
use crate::isa::op::{
    BitOp, ClmulOp, CountOp, IntWidth, MinMaxOp, PackOp, RotateOp, ShiftAddOp, SingleBitOp,
};

fn double(op: BitOp, a: u64, b: u64) -> u64 {
    execute(op, a, b, IntWidth::Double)
}

fn word(op: BitOp, a: u64, b: u64) -> u64 {
    execute(op, a, b, IntWidth::Word)
}

/// A register whose low word, `0x8000_0001`, is negative as a word and whose
/// upper word is all ones, so the `.uw` forms visibly drop it.
const NEGATIVE_LOW_WORD: u64 = 0xFFFF_FFFF_8000_0001;

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

#[test]
fn sh_n_add_shifts_rs1_left_by_n_before_adding() {
    assert_eq!(double(BitOp::ShiftAdd(ShiftAddOp::Sh1Add), 3, 10), 16);
    assert_eq!(double(BitOp::ShiftAdd(ShiftAddOp::Sh2Add), 3, 10), 22);
    assert_eq!(double(BitOp::ShiftAdd(ShiftAddOp::Sh3Add), 3, 10), 34);
}

#[test]
fn uw_forms_zero_extend_the_low_word_of_rs1() {
    let shift_add = |op, b| double(BitOp::ShiftAdd(op), NEGATIVE_LOW_WORD, b);

    assert_eq!(shift_add(ShiftAddOp::AddUw, 1), 0x8000_0002);
    assert_eq!(shift_add(ShiftAddOp::Sh1AddUw, 0), 0x1_0000_0002);
    assert_eq!(shift_add(ShiftAddOp::Sh2AddUw, 0), 0x2_0000_0004);
    assert_eq!(shift_add(ShiftAddOp::Sh3AddUw, 0), 0x4_0000_0008);
}

#[test]
fn slli_uw_shifts_the_zero_extended_word_by_six_bits_of_the_amount() {
    let slli_uw = |shamt| double(BitOp::ShiftAdd(ShiftAddOp::SlliUw), NEGATIVE_LOW_WORD, shamt);

    assert_eq!(slli_uw(4), 0x8_0000_0010);
    assert_eq!(slli_uw(64 + 4), 0x8_0000_0010);
}

#[test]
fn logic_with_negate_inverts_rs2_or_the_result() {
    let (a, b) = (0b1100, 0b1010);

    assert_eq!(double(BitOp::Andn, a, b), 0b0100);
    assert_eq!(double(BitOp::Orn, a, b), !0b0010);
    assert_eq!(double(BitOp::Xnor, a, b), !0b0110);
}

#[test]
fn counts_over_the_whole_register() {
    let count = |op, a| double(BitOp::Count(op), a, 0);

    assert_eq!(count(CountOp::Clz, 1 << 40), 23);
    assert_eq!(count(CountOp::Clz, 0), 64);
    assert_eq!(count(CountOp::Ctz, 1 << 40), 40);
    assert_eq!(count(CountOp::Cpop, 0xFF00_0000_0000_000F), 12);
}

#[test]
fn word_counts_ignore_the_upper_word() {
    let count = |op, a| word(BitOp::Count(op), a, 0);

    assert_eq!(count(CountOp::Clz, 0xFFFF_FFFF_0000_0100), 23);
    assert_eq!(count(CountOp::Ctz, 0x1_0000_0000), 32);
    assert_eq!(count(CountOp::Cpop, 0xFFFF_FFFF_0000_00F0), 4);
}

#[test]
fn sext_b_and_sext_h_extend_the_low_byte_and_half() {
    assert_eq!(double(BitOp::SextB, 0x80, 0), 0xFFFF_FFFF_FFFF_FF80);
    assert_eq!(double(BitOp::SextB, 0x17F, 0), 0x7F);
    assert_eq!(double(BitOp::SextH, 0x8000, 0), 0xFFFF_FFFF_FFFF_8000);
}

#[test]
fn rotates_wrap_the_whole_register_by_six_bits_of_rs2() {
    let rotate = |op, a, b| double(BitOp::Rotate(op), a, b);

    assert_eq!(rotate(RotateOp::Rol, 0x8000_0000_0000_0001, 1), 0b11);
    assert_eq!(rotate(RotateOp::Ror, 0b11, 64 + 1), 0x8000_0000_0000_0001);
}

#[test]
fn word_rotates_wrap_the_low_word_by_five_bits_and_sign_extend() {
    let rotate = |op, a, b| word(BitOp::Rotate(op), a, b);

    assert_eq!(rotate(RotateOp::Rol, 0x4000_0000, 32 + 1), 0xFFFF_FFFF_8000_0000);
    assert_eq!(rotate(RotateOp::Ror, 0b1, 1), 0xFFFF_FFFF_8000_0000);
}

#[test]
fn rev8_reverses_the_byte_order() {
    assert_eq!(double(BitOp::Rev8, 0x0102_0304_0506_0708, 0), 0x0807_0605_0403_0201);
}

#[test]
fn single_bit_ops_act_on_the_bit_rs2_numbers() {
    let single = |op, a, b| double(BitOp::SingleBit(op), a, b);

    assert_eq!(single(SingleBitOp::Bset, 0, 63), 1 << 63);
    assert_eq!(single(SingleBitOp::Bclr, u64::MAX, 0), u64::MAX - 1);
    assert_eq!(single(SingleBitOp::Binv, 0b101, 2), 0b001);
    assert_eq!(single(SingleBitOp::Bext, 0b100, 2), 1);
    assert_eq!(single(SingleBitOp::Bext, 0b100, 1), 0);
}

#[test]
fn single_bit_ops_take_the_bit_number_modulo_the_width() {
    assert_eq!(double(BitOp::SingleBit(SingleBitOp::Bset), 0, 64 + 3), 0b1000);
    assert_eq!(word(BitOp::SingleBit(SingleBitOp::Bset), 0, 32 + 1), 0b10);
}

#[test]
fn word_pack_joins_low_halves_and_sign_extends() {
    assert_eq!(word(BitOp::Pack(PackOp::Pack), 0x1234, 0x8765), 0xFFFF_FFFF_8765_1234);
}

#[test]
fn double_min_max_order_signed_and_unsigned() {
    let min_max = |op| double(BitOp::MinMax(op), u64::MAX, 1);

    assert_eq!(min_max(MinMaxOp::Max), 1);
    assert_eq!(min_max(MinMaxOp::Min), u64::MAX);
    assert_eq!(min_max(MinMaxOp::Maxu), u64::MAX);
    assert_eq!(min_max(MinMaxOp::Minu), 1);
}

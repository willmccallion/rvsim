//! ALU Logic & Comparison Operation Tests — Phase 1.1
//!
//! Deterministic edge-case tests for RISC-V bitwise logic (AND, OR, XOR)
//! and set-less-than comparisons (SLT, SLTU). Each operation group has 20+
//! test vectors covering:
//!   - Identity / annihilation laws
//!   - Alternating and single-bit patterns
//!   - Sign boundaries for comparisons
//!   - RV32 vs RV64 comparison semantics
//!
//! Reference: RISC-V ISA Specification, Volume I, Chapter 2.4.
//!
//! Note: AND, OR, XOR have no *W variants in the base ISA — they always
//! operate on full XLEN bits. SLT/SLTU likewise have no *W variants.

use crate::exec::compute::alu;
use crate::isa::op::{IntAluOp, IntWidth, LogicOp};

const ZERO: u64 = 0;
const ONE: u64 = 1;
const NEG1: u64 = u64::MAX; // 0xFFFF_FFFF_FFFF_FFFF

const I64_MAX: u64 = i64::MAX as u64; // 0x7FFF_FFFF_FFFF_FFFF
const I64_MIN: u64 = i64::MIN as u64; // 0x8000_0000_0000_0000
const U64_MAX: u64 = u64::MAX;

const ALTERNATING_A: u64 = 0xAAAA_AAAA_AAAA_AAAA;
const ALTERNATING_5: u64 = 0x5555_5555_5555_5555;
const LOW_BYTE: u64 = 0xFF;
const HIGH_BYTE: u64 = 0xFF00_0000_0000_0000;

fn alu(op: impl Into<IntAluOp>, a: u64, b: u64, width: IntWidth) -> u64 {
    alu::execute(op.into(), a, b, width)
}

const W: IntWidth = IntWidth::Word;
const D: IntWidth = IntWidth::Double;

#[test]
fn and_identity() {
    // x & 0xFFFF... = x
    assert_eq!(alu(LogicOp::And, 0xDEAD_BEEF_CAFE_BABE, NEG1, D), 0xDEAD_BEEF_CAFE_BABE);
}

#[test]
fn and_annihilation() {
    // x & 0 = 0
    assert_eq!(alu(LogicOp::And, 0xDEAD_BEEF_CAFE_BABE, ZERO, D), 0);
}

#[test]
fn and_idempotent() {
    // x & x = x
    let x = 0xDEAD_BEEF_CAFE_BABE;
    assert_eq!(alu(LogicOp::And, x, x, D), x);
}

#[test]
fn and_complement() {
    // x & ~x = 0
    assert_eq!(alu(LogicOp::And, ALTERNATING_A, ALTERNATING_5, D), 0);
}

#[test]
fn and_single_bit_mask() {
    // Extract bit 31
    assert_eq!(alu(LogicOp::And, 0xFFFF_FFFF, 1 << 31, D), 1 << 31);
    assert_eq!(alu(LogicOp::And, 0x7FFF_FFFF, 1 << 31, D), 0);
}

#[test]
fn and_byte_extraction() {
    assert_eq!(alu(LogicOp::And, 0x1234_5678_9ABC_DEF0, LOW_BYTE, D), 0xF0);
}

#[test]
fn and_high_byte_extraction() {
    assert_eq!(alu(LogicOp::And, 0xAB00_0000_0000_0000, HIGH_BYTE, D), 0xAB00_0000_0000_0000);
}

#[test]
fn and_commutative() {
    let a = 0x1234_5678;
    let b = 0xFF00_FF00;
    assert_eq!(alu(LogicOp::And, a, b, D), alu(LogicOp::And, b, a, D));
}

#[test]
fn and_all_ones() {
    assert_eq!(alu(LogicOp::And, NEG1, NEG1, D), NEG1);
}

#[test]
fn and_all_zeros() {
    assert_eq!(alu(LogicOp::And, ZERO, ZERO, D), ZERO);
}

#[test]
fn or_identity() {
    // x | 0 = x
    assert_eq!(alu(LogicOp::Or, 0xDEAD_BEEF_CAFE_BABE, ZERO, D), 0xDEAD_BEEF_CAFE_BABE);
}

#[test]
fn or_annihilation() {
    // x | 0xFFFF... = 0xFFFF...
    assert_eq!(alu(LogicOp::Or, 0xDEAD_BEEF_CAFE_BABE, NEG1, D), NEG1);
}

#[test]
fn or_idempotent() {
    let x = 0xDEAD_BEEF_CAFE_BABE;
    assert_eq!(alu(LogicOp::Or, x, x, D), x);
}

#[test]
fn or_complement_gives_all_ones() {
    assert_eq!(alu(LogicOp::Or, ALTERNATING_A, ALTERNATING_5, D), NEG1);
}

#[test]
fn or_single_bit_set() {
    assert_eq!(alu(LogicOp::Or, ZERO, 1 << 63, D), 1 << 63);
}

#[test]
fn or_merge_disjoint_fields() {
    let low = 0x0000_0000_FFFF_FFFF_u64;
    let high = 0xFFFF_FFFF_0000_0000_u64;
    assert_eq!(alu(LogicOp::Or, low, high, D), NEG1);
}

#[test]
fn or_commutative() {
    let a = 0x1234_5678;
    let b = 0xFF00_FF00;
    assert_eq!(alu(LogicOp::Or, a, b, D), alu(LogicOp::Or, b, a, D));
}

#[test]
fn or_all_zeros() {
    assert_eq!(alu(LogicOp::Or, ZERO, ZERO, D), ZERO);
}

#[test]
fn or_all_ones() {
    assert_eq!(alu(LogicOp::Or, NEG1, NEG1, D), NEG1);
}

#[test]
fn xor_self_is_zero() {
    let x = 0xDEAD_BEEF_CAFE_BABE;
    assert_eq!(alu(LogicOp::Xor, x, x, D), 0);
}

#[test]
fn xor_identity() {
    // x ^ 0 = x
    assert_eq!(alu(LogicOp::Xor, 0xDEAD_BEEF_CAFE_BABE, ZERO, D), 0xDEAD_BEEF_CAFE_BABE);
}

#[test]
fn xor_all_ones_is_bitwise_not() {
    assert_eq!(alu(LogicOp::Xor, ALTERNATING_A, NEG1, D), ALTERNATING_5);
}

#[test]
fn xor_complement() {
    assert_eq!(alu(LogicOp::Xor, ALTERNATING_A, ALTERNATING_5, D), NEG1);
}

#[test]
fn xor_commutative() {
    let a = 0x1234_5678_9ABC_DEF0;
    let b = 0xFEDC_BA98_7654_3210;
    assert_eq!(alu(LogicOp::Xor, a, b, D), alu(LogicOp::Xor, b, a, D));
}

#[test]
fn xor_associative() {
    let a = 0x1111_1111_1111_1111;
    let b = 0x2222_2222_2222_2222;
    let c = 0x3333_3333_3333_3333;
    let lhs = alu(LogicOp::Xor, alu(LogicOp::Xor, a, b, D), c, D);
    let rhs = alu(LogicOp::Xor, a, alu(LogicOp::Xor, b, c, D), D);
    assert_eq!(lhs, rhs);
}

#[test]
fn xor_single_bit_toggle() {
    assert_eq!(alu(LogicOp::Xor, 0b1010, 0b0001, D), 0b1011);
    assert_eq!(alu(LogicOp::Xor, 0b1011, 0b0001, D), 0b1010);
}

#[test]
fn xor_double_application_restores() {
    let x = 0xDEAD_BEEF_CAFE_BABE;
    let k = 0x1234_5678_9ABC_DEF0;
    let encrypted = alu(LogicOp::Xor, x, k, D);
    let decrypted = alu(LogicOp::Xor, encrypted, k, D);
    assert_eq!(decrypted, x);
}

#[test]
fn xor_all_zeros() {
    assert_eq!(alu(LogicOp::Xor, ZERO, ZERO, D), ZERO);
}

#[test]
fn xor_all_ones() {
    assert_eq!(alu(LogicOp::Xor, NEG1, NEG1, D), ZERO);
}

/// Verify: ~(a & b) == (~a) | (~b)
#[test]
fn de_morgan_and() {
    let a = 0xDEAD_BEEF_CAFE_BABE;
    let b = 0x1234_5678_9ABC_DEF0;
    let not_and = !alu(LogicOp::And, a, b, D);
    let or_nots = alu(LogicOp::Or, !a, !b, D);
    assert_eq!(not_and, or_nots);
}

/// Verify: ~(a | b) == (~a) & (~b)
#[test]
fn de_morgan_or() {
    let a = 0xDEAD_BEEF_CAFE_BABE;
    let b = 0x1234_5678_9ABC_DEF0;
    let not_or = !alu(LogicOp::Or, a, b, D);
    let and_nots = alu(LogicOp::And, !a, !b, D);
    assert_eq!(not_or, and_nots);
}

#[test]
fn slt_rv64_equal_values() {
    assert_eq!(alu(LogicOp::Slt, 42, 42, D), 0);
}

#[test]
fn slt_rv64_less_than() {
    assert_eq!(alu(LogicOp::Slt, 5, 10, D), 1);
}

#[test]
fn slt_rv64_greater_than() {
    assert_eq!(alu(LogicOp::Slt, 10, 5, D), 0);
}

#[test]
fn slt_rv64_negative_less_than_positive() {
    assert_eq!(alu(LogicOp::Slt, NEG1, ONE, D), 1);
}

#[test]
fn slt_rv64_positive_not_less_than_negative() {
    assert_eq!(alu(LogicOp::Slt, ONE, NEG1, D), 0);
}

#[test]
fn slt_rv64_min_less_than_max() {
    assert_eq!(alu(LogicOp::Slt, I64_MIN, I64_MAX, D), 1);
}

#[test]
fn slt_rv64_max_not_less_than_min() {
    assert_eq!(alu(LogicOp::Slt, I64_MAX, I64_MIN, D), 0);
}

#[test]
fn slt_rv64_min_less_than_zero() {
    assert_eq!(alu(LogicOp::Slt, I64_MIN, ZERO, D), 1);
}

#[test]
fn slt_rv64_zero_not_less_than_min() {
    assert_eq!(alu(LogicOp::Slt, ZERO, I64_MIN, D), 0);
}

#[test]
fn slt_rv64_neg1_less_than_0() {
    assert_eq!(alu(LogicOp::Slt, NEG1, ZERO, D), 1);
}

#[test]
fn slt_rv64_zero_not_less_than_zero() {
    assert_eq!(alu(LogicOp::Slt, ZERO, ZERO, D), 0);
}

/// Signed comparison: 0x8000...0 is negative (i64::MIN).
#[test]
fn slt_rv64_high_bit_is_negative() {
    assert_eq!(alu(LogicOp::Slt, I64_MIN, ONE, D), 1);
}

/// Adjacent values near zero.
#[test]
fn slt_rv64_adjacent_near_zero() {
    assert_eq!(alu(LogicOp::Slt, NEG1, ZERO, D), 1); // -1 < 0
    assert_eq!(alu(LogicOp::Slt, ZERO, ONE, D), 1); // 0 < 1
}

/// SLT in 32-bit mode should compare as i32 values.
#[test]
fn slt_rv32_basic() {
    assert_eq!(alu(LogicOp::Slt, 5, 10, W), 1);
    assert_eq!(alu(LogicOp::Slt, 10, 5, W), 0);
}

/// In RV32 mode, 0x8000_0000 is i32::MIN (negative).
#[test]
fn slt_rv32_high_bit_is_negative() {
    assert_eq!(alu(LogicOp::Slt, 0x8000_0000, ONE, W), 1);
}

/// In RV32 mode, upper 32 bits of the 64-bit operand should be ignored.
#[test]
fn slt_rv32_ignores_upper_bits() {
    assert_eq!(alu(LogicOp::Slt, 0xFFFF_FFFF_0000_0005, 0x0000_0000_0000_000A, W), 1);
}

/// 0xFFFF_FFFF in i32 is -1.
#[test]
fn slt_rv32_u32_max_is_negative_one() {
    assert_eq!(alu(LogicOp::Slt, 0xFFFF_FFFF, ZERO, W), 1);
}

#[test]
fn sltu_rv64_equal_values() {
    assert_eq!(alu(LogicOp::Sltu, 42, 42, D), 0);
}

#[test]
fn sltu_rv64_less_than() {
    assert_eq!(alu(LogicOp::Sltu, 5, 10, D), 1);
}

#[test]
fn sltu_rv64_greater_than() {
    assert_eq!(alu(LogicOp::Sltu, 10, 5, D), 0);
}

/// Unsigned: 0xFFFF... is the largest value, not -1.
#[test]
fn sltu_rv64_max_not_less_than_anything() {
    assert_eq!(alu(LogicOp::Sltu, U64_MAX, ONE, D), 0);
}

#[test]
fn sltu_rv64_zero_less_than_anything_nonzero() {
    assert_eq!(alu(LogicOp::Sltu, ZERO, ONE, D), 1);
    assert_eq!(alu(LogicOp::Sltu, ZERO, U64_MAX, D), 1);
}

/// Key difference from SLT: 0x8000...0 is a large positive unsigned value.
#[test]
fn sltu_rv64_high_bit_is_large_positive() {
    assert_eq!(alu(LogicOp::Sltu, I64_MIN, ONE, D), 0); // 2^63 > 1
    assert_eq!(alu(LogicOp::Sltu, ONE, I64_MIN, D), 1); // 1 < 2^63
}

#[test]
fn sltu_rv64_zero_not_less_than_zero() {
    assert_eq!(alu(LogicOp::Sltu, ZERO, ZERO, D), 0);
}

#[test]
fn sltu_rv64_max_minus_one_less_than_max() {
    assert_eq!(alu(LogicOp::Sltu, U64_MAX - 1, U64_MAX, D), 1);
}

/// RISC-V idiom: SLTU rd, x0, rs1 sets rd=1 if rs1 != 0 (SNEZ pseudo-op).
#[test]
fn sltu_rv64_snez_idiom() {
    assert_eq!(alu(LogicOp::Sltu, ZERO, 42, D), 1);
    assert_eq!(alu(LogicOp::Sltu, ZERO, ZERO, D), 0);
    assert_eq!(alu(LogicOp::Sltu, ZERO, NEG1, D), 1);
}

/// SLTU in 32-bit mode should compare as u32 values.
#[test]
fn sltu_rv32_basic() {
    assert_eq!(alu(LogicOp::Sltu, 5, 10, W), 1);
    assert_eq!(alu(LogicOp::Sltu, 10, 5, W), 0);
}

/// In RV32 mode, 0x8000_0000 is a large positive u32.
#[test]
fn sltu_rv32_high_bit_is_large() {
    assert_eq!(alu(LogicOp::Sltu, 0x8000_0000, ONE, W), 0);
    assert_eq!(alu(LogicOp::Sltu, ONE, 0x8000_0000, W), 1);
}

/// RV32 SLTU ignores upper 32 bits.
#[test]
fn sltu_rv32_ignores_upper_bits() {
    assert_eq!(alu(LogicOp::Sltu, 0xDEAD_0000_0000_0005, 0xBEEF_0000_0000_000A, W), 1);
}

#[test]
fn sltu_rv32_u32_max() {
    assert_eq!(alu(LogicOp::Sltu, 0xFFFF_FFFF, ZERO, W), 0);
    assert_eq!(alu(LogicOp::Sltu, ZERO, 0xFFFF_FFFF, W), 1);
}

/// The same bit pattern gives opposite results for SLT vs SLTU
/// when one operand has the high bit set.
#[test]
fn slt_vs_sltu_sign_bit_distinction() {
    // SLT:  i64::MIN < 1 → true (negative < positive)
    assert_eq!(alu(LogicOp::Slt, I64_MIN, ONE, D), 1);
    // SLTU: 2^63 < 1 → false (large unsigned > small)
    assert_eq!(alu(LogicOp::Sltu, I64_MIN, ONE, D), 0);
}

/// NEG1 = 0xFFFF... is -1 signed but u64::MAX unsigned.
#[test]
fn slt_vs_sltu_neg1_distinction() {
    // SLT:  -1 < 0 → true
    assert_eq!(alu(LogicOp::Slt, NEG1, ZERO, D), 1);
    // SLTU: u64::MAX < 0 → false
    assert_eq!(alu(LogicOp::Sltu, NEG1, ZERO, D), 0);
}

/// Verify AND works correctly at every single bit position.
#[test]
fn and_every_bit_position() {
    for bit in 0..64 {
        let mask = 1u64 << bit;
        assert_eq!(alu(LogicOp::And, NEG1, mask, D), mask, "AND failed at bit position {bit}");
        assert_eq!(
            alu(LogicOp::And, !mask, mask, D),
            0,
            "AND complement failed at bit position {bit}"
        );
    }
}

/// Verify OR can set every single bit position.
#[test]
fn or_every_bit_position() {
    for bit in 0..64 {
        let mask = 1u64 << bit;
        assert_eq!(alu(LogicOp::Or, ZERO, mask, D), mask, "OR failed at bit position {bit}");
    }
}

/// Verify XOR can toggle every single bit position.
#[test]
fn xor_every_bit_position() {
    for bit in 0..64 {
        let mask = 1u64 << bit;
        assert_eq!(alu(LogicOp::Xor, NEG1, mask, D), !mask, "XOR failed at bit position {bit}");
    }
}

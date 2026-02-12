//! ALU Shift Operation Tests — Phase 1.1
//!
//! Deterministic edge-case tests for RISC-V shift operations:
//!   SLL/SLLW  — Shift Left Logical
//!   SRL/SRLW  — Shift Right Logical
//!   SRA/SRAW  — Shift Right Arithmetic
//!
//! Each operation group has 20+ test vectors covering:
//!   - Boundary shift amounts (0, 1, 31, 32, 63)
//!   - Shift amount masking (only low 6 bits for RV64, low 5 for RV32)
//!   - Sign-extension behavior for SRA
//!   - *W variant sign-extension of results
//!
//! Reference: RISC-V ISA Specification, Volume I, Chapter 2.4.

use riscv_core::core::pipeline::signals::AluOp;
use riscv_core::core::units::alu::Alu;

// ─── Constants ───────────────────────────────────────────────────────────────

const ZERO: u64 = 0;
const ONE: u64 = 1;
const NEG1: u64 = u64::MAX; // 0xFFFF_FFFF_FFFF_FFFF

const I64_MAX: u64 = i64::MAX as u64; // 0x7FFF_FFFF_FFFF_FFFF
const I64_MIN: u64 = i64::MIN as u64; // 0x8000_0000_0000_0000

const I32_MIN_SEXT: u64 = i32::MIN as i64 as u64; // 0xFFFF_FFFF_8000_0000

// ─── Helper ──────────────────────────────────────────────────────────────────

fn alu(op: AluOp, a: u64, b: u64, is32: bool) -> u64 {
    Alu::execute(op, a, b, 0, is32)
}

fn sext32(val: u32) -> u64 {
    val as i32 as i64 as u64
}

// ═════════════════════════════════════════════════════════════════════════════
//  SLL (Shift Left Logical — RV64)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn sll_rv64_shift_by_zero() {
    assert_eq!(alu(AluOp::Sll, 0xDEAD_BEEF, ZERO, false), 0xDEAD_BEEF);
}

#[test]
fn sll_rv64_shift_by_one() {
    assert_eq!(alu(AluOp::Sll, ONE, ONE, false), 2);
}

#[test]
fn sll_rv64_shift_by_63() {
    // 1 << 63 = 0x8000_0000_0000_0000
    assert_eq!(alu(AluOp::Sll, ONE, 63, false), I64_MIN);
}

#[test]
fn sll_rv64_shift_all_ones() {
    // 0xFFFF... << 1 = 0xFFFF...E
    assert_eq!(alu(AluOp::Sll, NEG1, ONE, false), NEG1 - 1);
}

#[test]
fn sll_rv64_shift_out_all_bits() {
    // 0xFFFF... << 63 = 0x8000...0
    assert_eq!(alu(AluOp::Sll, NEG1, 63, false), I64_MIN);
}

#[test]
fn sll_rv64_zero_shifted() {
    assert_eq!(alu(AluOp::Sll, ZERO, 32, false), 0);
}

/// RISC-V spec: Only the low 6 bits of the shift amount are used (RV64).
/// A shift of 64 is masked to 0, so the value is unchanged.
#[test]
fn sll_rv64_shift_amount_masked_to_6_bits() {
    // shift = 64 → masked to 0
    assert_eq!(alu(AluOp::Sll, 42, 64, false), 42);
    // shift = 65 → masked to 1
    assert_eq!(alu(AluOp::Sll, 42, 65, false), 84);
    // shift = 127 → masked to 63
    assert_eq!(alu(AluOp::Sll, ONE, 127, false), I64_MIN);
}

/// Shift amounts with upper bits set should be ignored.
#[test]
fn sll_rv64_upper_bits_of_shift_ignored() {
    // b = 0xFFFF_FFFF_FFFF_FF01 → low 6 bits = 1
    assert_eq!(alu(AluOp::Sll, ONE, 0xFFFF_FFFF_FFFF_FF01, false), 2);
}

#[test]
fn sll_rv64_power_of_two_generation() {
    for i in 0..64 {
        assert_eq!(
            alu(AluOp::Sll, ONE, i, false),
            1u64 << i,
            "SLL failed: 1 << {i}"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  SLLW (Shift Left Logical — RV32)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn sllw_shift_by_zero() {
    // Result is sign-extended: 0xDEAD_BEEF has bit 31 set → negative
    assert_eq!(
        alu(AluOp::Sll, 0xDEAD_BEEF, ZERO, true),
        sext32(0xDEAD_BEEF)
    );
}

#[test]
fn sllw_shift_by_one() {
    assert_eq!(alu(AluOp::Sll, ONE, ONE, true), 2);
}

#[test]
fn sllw_shift_by_31() {
    // 1 << 31 = 0x8000_0000 → sign-extended = 0xFFFF_FFFF_8000_0000
    assert_eq!(alu(AluOp::Sll, ONE, 31, true), I32_MIN_SEXT);
}

/// RISC-V spec: Only the low 5 bits of the shift amount are used (RV32).
#[test]
fn sllw_shift_amount_masked_to_5_bits() {
    // shift = 32 → masked to 0
    assert_eq!(alu(AluOp::Sll, 42, 32, true), sext32(42));
    // shift = 33 → masked to 1
    assert_eq!(alu(AluOp::Sll, 42, 33, true), sext32(84));
}

#[test]
fn sllw_ignores_upper_32_bits_of_operand() {
    // Upper bits of `a` are discarded; only a[31:0] is shifted.
    assert_eq!(alu(AluOp::Sll, 0xFFFF_FFFF_0000_0001, 1, true), 2);
}

#[test]
fn sllw_result_sign_extends_when_bit31_set() {
    // 0x4000_0000 << 1 = 0x8000_0000 → sign-extended
    assert_eq!(alu(AluOp::Sll, 0x4000_0000, 1, true), I32_MIN_SEXT);
}

#[test]
fn sllw_all_ones_shift_by_1() {
    // 0xFFFF_FFFF << 1 = 0xFFFF_FFFE → sign-extended
    assert_eq!(alu(AluOp::Sll, 0xFFFF_FFFF, 1, true), sext32(0xFFFF_FFFE));
}

// ═════════════════════════════════════════════════════════════════════════════
//  SRL (Shift Right Logical — RV64)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn srl_rv64_shift_by_zero() {
    assert_eq!(alu(AluOp::Srl, 0xDEAD_BEEF, ZERO, false), 0xDEAD_BEEF);
}

#[test]
fn srl_rv64_shift_by_one() {
    assert_eq!(alu(AluOp::Srl, 2, ONE, false), 1);
}

#[test]
fn srl_rv64_shift_by_63() {
    // 0x8000...0 >> 63 = 1
    assert_eq!(alu(AluOp::Srl, I64_MIN, 63, false), 1);
}

/// SRL fills with zeros from the left (logical, not arithmetic).
#[test]
fn srl_rv64_fills_with_zeros() {
    // 0xFFFF... >> 1 = 0x7FFF...
    assert_eq!(alu(AluOp::Srl, NEG1, ONE, false), I64_MAX);
}

#[test]
fn srl_rv64_all_ones_shift_by_63() {
    assert_eq!(alu(AluOp::Srl, NEG1, 63, false), 1);
}

#[test]
fn srl_rv64_zero_shifted() {
    assert_eq!(alu(AluOp::Srl, ZERO, 32, false), 0);
}

/// Shift amount masking: only low 6 bits.
#[test]
fn srl_rv64_shift_amount_masked() {
    // shift = 64 → masked to 0
    assert_eq!(alu(AluOp::Srl, 42, 64, false), 42);
    // shift = 65 → masked to 1
    assert_eq!(alu(AluOp::Srl, 42, 65, false), 21);
}

#[test]
fn srl_rv64_upper_bits_of_shift_ignored() {
    assert_eq!(alu(AluOp::Srl, 0x100, 0xFFFF_FFFF_FFFF_FF04, false), 0x10);
}

/// Verify SRL produces powers of two when shifting all-ones right.
#[test]
fn srl_rv64_successive_shifts() {
    for i in 0..64 {
        let expected = NEG1 >> i;
        assert_eq!(
            alu(AluOp::Srl, NEG1, i, false),
            expected,
            "SRL failed: 0xFFFF... >> {i}"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  SRLW (Shift Right Logical — RV32)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn srlw_shift_by_zero() {
    // 0xDEAD_BEEF as u32 >> 0 = 0xDEAD_BEEF → sign-extended (bit 31 set)
    assert_eq!(
        alu(AluOp::Srl, 0xDEAD_BEEF, ZERO, true),
        sext32(0xDEAD_BEEF)
    );
}

#[test]
fn srlw_shift_by_one() {
    // 0x8000_0000 >> 1 = 0x4000_0000 (logical, zero-fills)
    assert_eq!(alu(AluOp::Srl, 0x8000_0000, ONE, true), sext32(0x4000_0000));
}

#[test]
fn srlw_shift_by_31() {
    // 0x8000_0000 >> 31 = 1
    assert_eq!(alu(AluOp::Srl, 0x8000_0000, 31, true), 1);
}

/// SRLW shift amount masked to 5 bits.
#[test]
fn srlw_shift_amount_masked_to_5_bits() {
    // shift = 32 → masked to 0
    assert_eq!(alu(AluOp::Srl, 0xDEAD_BEEF, 32, true), sext32(0xDEAD_BEEF));
}

#[test]
fn srlw_ignores_upper_32_bits_of_operand() {
    // Only a[31:0] = 0x8000_0000 is shifted
    assert_eq!(
        alu(AluOp::Srl, 0xFFFF_FFFF_8000_0000, 1, true),
        sext32(0x4000_0000)
    );
}

/// SRLW: zero-fill + sign-extension interaction.
/// After shifting 0xFFFF_FFFF right by 1, we get 0x7FFF_FFFF (bit 31 = 0).
/// Sign-extended: 0x0000_0000_7FFF_FFFF.
#[test]
fn srlw_zero_fill_clears_sign_bit() {
    assert_eq!(alu(AluOp::Srl, 0xFFFF_FFFF, 1, true), sext32(0x7FFF_FFFF));
}

/// All ones shifted right by 31 leaves just bit 0.
#[test]
fn srlw_all_ones_shift_by_31() {
    assert_eq!(alu(AluOp::Srl, 0xFFFF_FFFF, 31, true), 1);
}

// ═════════════════════════════════════════════════════════════════════════════
//  SRA (Shift Right Arithmetic — RV64)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn sra_rv64_shift_by_zero() {
    assert_eq!(alu(AluOp::Sra, 0xDEAD_BEEF, ZERO, false), 0xDEAD_BEEF);
}

#[test]
fn sra_rv64_positive_shift() {
    // Positive value: SRA == SRL
    assert_eq!(alu(AluOp::Sra, 100, 2, false), 25);
}

/// SRA fills with copies of the sign bit.
#[test]
fn sra_rv64_negative_fills_with_ones() {
    // 0xFFFF... >> 1 = 0xFFFF... (all ones stays all ones)
    assert_eq!(alu(AluOp::Sra, NEG1, ONE, false), NEG1);
}

#[test]
fn sra_rv64_negative_shift_by_63() {
    // i64::MIN >> 63 = -1 (sign bit propagates everywhere)
    assert_eq!(alu(AluOp::Sra, I64_MIN, 63, false), NEG1);
}

#[test]
fn sra_rv64_positive_shift_by_63() {
    // i64::MAX >> 63 = 0 (positive, zero-fills)
    assert_eq!(alu(AluOp::Sra, I64_MAX, 63, false), 0);
}

/// SRA vs SRL: same positive value gives same result.
#[test]
fn sra_vs_srl_positive_equivalent() {
    let val = 0x0000_DEAD_BEEF_0000_u64;
    for shift in 0..64 {
        assert_eq!(
            alu(AluOp::Sra, val, shift, false),
            alu(AluOp::Srl, val, shift, false),
            "SRA != SRL for positive value at shift {shift}"
        );
    }
}

/// SRA vs SRL: negative value diverges at shift > 0.
#[test]
fn sra_vs_srl_negative_diverge() {
    // SRA: 0x8000...0 >> 1 = 0xC000...0 (sign-extends)
    assert_eq!(alu(AluOp::Sra, I64_MIN, 1, false), 0xC000_0000_0000_0000);
    // SRL: 0x8000...0 >> 1 = 0x4000...0 (zero-fills)
    assert_eq!(alu(AluOp::Srl, I64_MIN, 1, false), 0x4000_0000_0000_0000);
}

/// Shift amount masking.
#[test]
fn sra_rv64_shift_amount_masked() {
    // shift = 64 → masked to 0
    assert_eq!(alu(AluOp::Sra, I64_MIN, 64, false), I64_MIN);
}

/// Arithmetic right shift of -2 by 1 should give -1.
#[test]
fn sra_rv64_neg2_shift_by_1() {
    assert_eq!(alu(AluOp::Sra, -2i64 as u64, 1, false), NEG1);
}

/// Shift each bit position to verify sign-extension pattern.
#[test]
fn sra_rv64_progressive_shift_negative() {
    for i in 0..64 {
        let expected = (i64::MIN >> i) as u64;
        assert_eq!(
            alu(AluOp::Sra, I64_MIN, i, false),
            expected,
            "SRA failed: i64::MIN >> {i}"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  SRAW (Shift Right Arithmetic — RV32)
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn sraw_shift_by_zero() {
    // 0x8000_0000 as i32 >> 0 = 0x8000_0000 → sign-extended
    assert_eq!(alu(AluOp::Sra, 0x8000_0000, ZERO, true), I32_MIN_SEXT);
}

#[test]
fn sraw_positive_shift() {
    assert_eq!(alu(AluOp::Sra, 100, 2, true), 25);
}

/// SRAW: negative 32-bit value fills with ones.
#[test]
fn sraw_negative_fills_with_ones() {
    // 0x8000_0000 (i32::MIN) >> 1 = 0xC000_0000 → sign-extended
    assert_eq!(alu(AluOp::Sra, 0x8000_0000, 1, true), sext32(0xC000_0000));
}

#[test]
fn sraw_negative_shift_by_31() {
    // i32::MIN >> 31 = -1 → sign-extended = 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(alu(AluOp::Sra, 0x8000_0000, 31, true), NEG1);
}

#[test]
fn sraw_positive_shift_by_31() {
    // i32::MAX >> 31 = 0
    assert_eq!(alu(AluOp::Sra, 0x7FFF_FFFF, 31, true), 0);
}

/// SRAW shift amount masked to 5 bits.
#[test]
fn sraw_shift_amount_masked_to_5_bits() {
    // shift = 32 → masked to 0
    assert_eq!(alu(AluOp::Sra, 0x8000_0000, 32, true), I32_MIN_SEXT);
}

/// SRAW ignores upper 32 bits of operand — treats a as i32.
#[test]
fn sraw_ignores_upper_32_bits_of_operand() {
    // a = 0x0000_0001_8000_0000 → a[31:0] = 0x8000_0000 (negative as i32)
    assert_eq!(
        alu(AluOp::Sra, 0x0000_0001_8000_0000, 1, true),
        sext32(0xC000_0000)
    );
}

/// SRAW: all-ones stays all-ones regardless of shift.
#[test]
fn sraw_all_ones_stays_all_ones() {
    for shift in 0..32 {
        assert_eq!(
            alu(AluOp::Sra, 0xFFFF_FFFF, shift, true),
            NEG1,
            "SRAW -1 >> {shift} should remain -1"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  Cross-cutting: All *W shift results must be sign-extended
// ═════════════════════════════════════════════════════════════════════════════

/// Every *W shift must produce a result where bits [63:32] are all copies
/// of bit 31.
#[test]
fn all_w_shift_results_are_sign_extended() {
    let test_cases: Vec<(AluOp, u64, u64)> = vec![
        // SLLW
        (AluOp::Sll, 1, 0),
        (AluOp::Sll, 1, 31),           // 1 << 31 = 0x8000_0000 (negative)
        (AluOp::Sll, 0x4000_0000, 1),  // bit 31 set in result
        (AluOp::Sll, 0xFFFF_FFFF, 16), // upper bits shifted out
        // SRLW
        (AluOp::Srl, 0x8000_0000, 0),  // no shift, bit 31 set
        (AluOp::Srl, 0x8000_0000, 1),  // bit 31 cleared
        (AluOp::Srl, 0xFFFF_FFFF, 1),  // 0x7FFF_FFFF
        (AluOp::Srl, 0xFFFF_FFFF, 31), // 1
        // SRAW
        (AluOp::Sra, 0x8000_0000, 0),  // negative, no shift
        (AluOp::Sra, 0x8000_0000, 1),  // 0xC000_0000 (still negative)
        (AluOp::Sra, 0x8000_0000, 31), // -1
        (AluOp::Sra, 0x7FFF_FFFF, 1),  // positive
    ];

    for (op, a, b) in test_cases {
        let result = alu(op, a, b, true);
        let bit31 = (result >> 31) & 1;
        let upper = result >> 32;
        let expected_upper = if bit31 == 1 { 0xFFFF_FFFF } else { 0 };
        assert_eq!(
            upper, expected_upper,
            "Op {:?} with a={:#x}, b={}: result {:#018x} not sign-extended from bit 31",
            op, a, b, result
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  Shift idioms commonly used by compilers
// ═════════════════════════════════════════════════════════════════════════════

/// Multiply by power of 2 via SLL.
#[test]
fn sll_multiply_by_power_of_two() {
    assert_eq!(alu(AluOp::Sll, 7, 3, false), 56); // 7 * 8
}

/// Unsigned divide by power of 2 via SRL.
#[test]
fn srl_divide_by_power_of_two() {
    assert_eq!(alu(AluOp::Srl, 56, 3, false), 7); // 56 / 8
}

/// Signed divide by power of 2 via SRA (rounds toward -infinity, not zero).
#[test]
fn sra_signed_divide_rounds_toward_negative_infinity() {
    // -7 >> 1 = -4 (rounds toward -inf, not -3)
    assert_eq!(alu(AluOp::Sra, -7i64 as u64, 1, false), -4i64 as u64);
}

/// Extract byte N via SRL + AND (common pattern).
#[test]
fn srl_extract_byte() {
    let val = 0x1234_5678_9ABC_DEF0_u64;
    // Extract byte 3 (bits 31:24)
    let byte3 = alu(AluOp::Srl, val, 24, false) & 0xFF;
    assert_eq!(byte3, 0x9A);
}

//! ALU Arithmetic Operation Tests — Phase 1.1
//!
//! Deterministic edge-case tests for all RISC-V integer arithmetic operations
//! (RV64I + RV64M). Each operation group has 20+ test vectors covering:
//!   - Boundary values (0, 1, -1, MAX, MIN)
//!   - Overflow/underflow wrapping behavior
//!   - Signed/unsigned mixing
//!   - The DIVU/REMU casting bugs identified in Phase 0
//!
//! Reference: RISC-V ISA Specification, Volume I, Chapters 2.4 and 7.

use riscv_core::core::pipeline::signals::AluOp;
use riscv_core::core::units::alu::Alu;

// ─── Constants ───────────────────────────────────────────────────────────────
// Named constants for readability. Every magic number in a test vector should
// be traceable to an architectural boundary condition.

const ZERO: u64 = 0;
const ONE: u64 = 1;
const NEG1: u64 = -1i64 as u64; // 0xFFFF_FFFF_FFFF_FFFF

// RV64 signed boundaries
const I64_MAX: u64 = i64::MAX as u64; // 0x7FFF_FFFF_FFFF_FFFF
const I64_MIN: u64 = i64::MIN as u64; // 0x8000_0000_0000_0000

// RV64 unsigned boundary
const U64_MAX: u64 = u64::MAX; // 0xFFFF_FFFF_FFFF_FFFF

// RV32 signed boundaries (as 64-bit values)
const I32_MAX: u64 = i32::MAX as u64; // 0x0000_0000_7FFF_FFFF
const I32_MIN: u64 = i32::MIN as i64 as u64; // 0xFFFF_FFFF_8000_0000

// RV32 unsigned boundary
const U32_MAX: u64 = u32::MAX as u64; // 0x0000_0000_FFFF_FFFF

// Useful patterns
const ALTERNATING_A: u64 = 0xAAAA_AAAA_AAAA_AAAA;
const ALTERNATING_5: u64 = 0x5555_5555_5555_5555;
const HIGH_BIT_32: u64 = 0x8000_0000; // Bit 31 set

// ─── Helper ──────────────────────────────────────────────────────────────────

/// Execute an ALU operation. Thin wrapper to keep test lines short.
fn alu(op: AluOp, a: u64, b: u64, is32: bool) -> u64 {
    Alu::execute(op, a, b, 0, is32)
}

/// Sign-extend a 32-bit value to 64 bits (what every *W instruction must do).
fn sext32(val: u32) -> u64 {
    val as i32 as i64 as u64
}

// ═════════════════════════════════════════════════════════════════════════════
//  ADD / ADDW
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn add_rv64_zero_plus_zero() {
    assert_eq!(alu(AluOp::Add, ZERO, ZERO, false), 0);
}

#[test]
fn add_rv64_identity() {
    assert_eq!(alu(AluOp::Add, 42, ZERO, false), 42);
    assert_eq!(alu(AluOp::Add, ZERO, 42, false), 42);
}

#[test]
fn add_rv64_positive_plus_positive() {
    assert_eq!(alu(AluOp::Add, 100, 200, false), 300);
}

#[test]
fn add_rv64_negative_plus_negative() {
    // -5 + -3 = -8
    let neg5 = -5i64 as u64;
    let neg3 = -3i64 as u64;
    let neg8 = -8i64 as u64;
    assert_eq!(alu(AluOp::Add, neg5, neg3, false), neg8);
}

#[test]
fn add_rv64_positive_plus_negative() {
    // 10 + (-3) = 7
    assert_eq!(alu(AluOp::Add, 10, -3i64 as u64, false), 7);
}

#[test]
fn add_rv64_neg1_plus_1() {
    assert_eq!(alu(AluOp::Add, NEG1, ONE, false), 0);
}

#[test]
fn add_rv64_max_plus_1_wraps() {
    // Signed overflow: i64::MAX + 1 wraps to i64::MIN
    assert_eq!(alu(AluOp::Add, I64_MAX, ONE, false), I64_MIN);
}

#[test]
fn add_rv64_unsigned_max_plus_1_wraps() {
    // Unsigned overflow: u64::MAX + 1 wraps to 0
    assert_eq!(alu(AluOp::Add, U64_MAX, ONE, false), 0);
}

#[test]
fn add_rv64_min_plus_min() {
    // i64::MIN + i64::MIN wraps to 0
    assert_eq!(alu(AluOp::Add, I64_MIN, I64_MIN, false), 0);
}

#[test]
fn add_rv64_large_values() {
    assert_eq!(
        alu(
            AluOp::Add,
            0xDEAD_BEEF_CAFE_BABE,
            0x1111_1111_1111_1111,
            false
        ),
        0xDEAD_BEEF_CAFE_BABE_u64.wrapping_add(0x1111_1111_1111_1111)
    );
}

#[test]
fn addw_zero_plus_zero() {
    assert_eq!(alu(AluOp::Add, ZERO, ZERO, true), 0);
}

#[test]
fn addw_positive_plus_positive() {
    assert_eq!(alu(AluOp::Add, 100, 200, true), 300);
}

#[test]
fn addw_overflow_wraps_and_sign_extends() {
    // i32::MAX + 1 = 0x8000_0000, sign-extended = 0xFFFF_FFFF_8000_0000
    assert_eq!(alu(AluOp::Add, I32_MAX, ONE, true), I32_MIN);
}

#[test]
fn addw_negative_result_sign_extends() {
    // -1 + 0 in 32-bit = 0xFFFF_FFFF sign-extended to 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(alu(AluOp::Add, NEG1, ZERO, true), NEG1);
}

#[test]
fn addw_ignores_upper_32_bits_of_inputs() {
    // Upper bits of inputs should be ignored; result is sign-extended from bit 31.
    // 0xDEAD_0000_0000_0001 + 0xBEEF_0000_0000_0002 → ADDW should see 1 + 2 = 3
    assert_eq!(
        alu(
            AluOp::Add,
            0xDEAD_0000_0000_0001,
            0xBEEF_0000_0000_0002,
            true
        ),
        3
    );
}

#[test]
fn addw_u32_max_plus_1() {
    // 0xFFFF_FFFF + 1 = 0x1_0000_0000 → truncated to 32-bit = 0, sign-extended = 0
    assert_eq!(alu(AluOp::Add, U32_MAX, ONE, true), 0);
}

// ═════════════════════════════════════════════════════════════════════════════
//  SUB / SUBW
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn sub_rv64_zero_minus_zero() {
    assert_eq!(alu(AluOp::Sub, ZERO, ZERO, false), 0);
}

#[test]
fn sub_rv64_positive_minus_positive() {
    assert_eq!(alu(AluOp::Sub, 200, 100, false), 100);
}

#[test]
fn sub_rv64_zero_minus_one() {
    assert_eq!(alu(AluOp::Sub, ZERO, ONE, false), NEG1);
}

#[test]
fn sub_rv64_min_minus_one_wraps() {
    // i64::MIN - 1 wraps to i64::MAX
    assert_eq!(alu(AluOp::Sub, I64_MIN, ONE, false), I64_MAX);
}

#[test]
fn sub_rv64_zero_minus_min() {
    // 0 - i64::MIN = i64::MIN (wraps due to two's complement)
    assert_eq!(alu(AluOp::Sub, ZERO, I64_MIN, false), I64_MIN);
}

#[test]
fn sub_rv64_self_minus_self() {
    assert_eq!(alu(AluOp::Sub, 0xDEAD_BEEF, 0xDEAD_BEEF, false), 0);
}

#[test]
fn sub_rv64_negative_minus_negative() {
    // -5 - (-3) = -2
    assert_eq!(
        alu(AluOp::Sub, -5i64 as u64, -3i64 as u64, false),
        -2i64 as u64
    );
}

#[test]
fn subw_positive_result() {
    assert_eq!(alu(AluOp::Sub, 10, 3, true), 7);
}

#[test]
fn subw_negative_result_sign_extends() {
    // 3 - 10 = -7 → sign-extended
    assert_eq!(alu(AluOp::Sub, 3, 10, true), -7i64 as u64);
}

#[test]
fn subw_overflow_wraps_and_sign_extends() {
    // i32::MIN - 1 wraps to i32::MAX, sign-extended
    assert_eq!(alu(AluOp::Sub, I32_MIN, ONE, true), sext32(i32::MAX as u32));
}

#[test]
fn subw_ignores_upper_bits() {
    assert_eq!(
        alu(
            AluOp::Sub,
            0xFF00_0000_0000_000A,
            0xAB00_0000_0000_0003,
            true
        ),
        7
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  MUL / MULW
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn mul_rv64_zero_times_anything() {
    assert_eq!(alu(AluOp::Mul, ZERO, 12345, false), 0);
    assert_eq!(alu(AluOp::Mul, 12345, ZERO, false), 0);
}

#[test]
fn mul_rv64_identity() {
    assert_eq!(alu(AluOp::Mul, 42, ONE, false), 42);
    assert_eq!(alu(AluOp::Mul, ONE, 42, false), 42);
}

#[test]
fn mul_rv64_neg1_is_negate() {
    // x * -1 = -x
    assert_eq!(alu(AluOp::Mul, 42, NEG1, false), (-42i64) as u64);
}

#[test]
fn mul_rv64_neg1_times_neg1() {
    assert_eq!(alu(AluOp::Mul, NEG1, NEG1, false), 1);
}

#[test]
fn mul_rv64_positive_times_positive() {
    assert_eq!(alu(AluOp::Mul, 100, 200, false), 20_000);
}

#[test]
fn mul_rv64_overflow_wraps() {
    // Large multiply: result is lower 64 bits only
    let a = 0x1_0000_0000_u64; // 2^32
    let b = 0x1_0000_0000_u64;
    assert_eq!(alu(AluOp::Mul, a, b, false), 0); // 2^64 mod 2^64 = 0
}

#[test]
fn mul_rv64_max_times_2() {
    assert_eq!(alu(AluOp::Mul, I64_MAX, 2, false), I64_MAX.wrapping_mul(2));
}

#[test]
fn mulw_basic() {
    assert_eq!(alu(AluOp::Mul, 7, 6, true), 42);
}

#[test]
fn mulw_overflow_wraps_and_sign_extends() {
    // 0x7FFF_FFFF * 2 = 0xFFFF_FFFE → sign-extended = 0xFFFF_FFFF_FFFF_FFFE
    assert_eq!(alu(AluOp::Mul, I32_MAX, 2, true), sext32(0xFFFF_FFFE));
}

#[test]
fn mulw_neg1_times_neg1() {
    assert_eq!(alu(AluOp::Mul, NEG1, NEG1, true), 1);
}

#[test]
fn mulw_ignores_upper_bits() {
    assert_eq!(
        alu(
            AluOp::Mul,
            0xFFFF_FFFF_0000_0003,
            0xFFFF_FFFF_0000_0004,
            true
        ),
        12
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  MULH / MULHSU / MULHU
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn mulh_zero() {
    assert_eq!(alu(AluOp::Mulh, ZERO, 42, false), 0);
}

#[test]
fn mulh_small_values_no_high_bits() {
    // Small numbers: high 64 bits are 0
    assert_eq!(alu(AluOp::Mulh, 100, 200, false), 0);
}

#[test]
fn mulh_max_times_max() {
    // i64::MAX * i64::MAX: high bits of 128-bit result
    let expected = ((i64::MAX as i128 * i64::MAX as i128) >> 64) as u64;
    assert_eq!(alu(AluOp::Mulh, I64_MAX, I64_MAX, false), expected);
}

#[test]
fn mulh_neg1_times_neg1() {
    // (-1) * (-1) = 1, high 64 bits = 0
    assert_eq!(alu(AluOp::Mulh, NEG1, NEG1, false), 0);
}

#[test]
fn mulh_neg1_times_positive() {
    // (-1) * 1 = -1, high 64 bits = -1 (0xFFFF_FFFF_FFFF_FFFF)
    assert_eq!(alu(AluOp::Mulh, NEG1, ONE, false), NEG1);
}

#[test]
fn mulh_min_times_min() {
    let expected = ((i64::MIN as i128 * i64::MIN as i128) >> 64) as u64;
    assert_eq!(alu(AluOp::Mulh, I64_MIN, I64_MIN, false), expected);
}

#[test]
fn mulhsu_positive_times_positive() {
    assert_eq!(alu(AluOp::Mulhsu, 100, 200, false), 0);
}

#[test]
fn mulhsu_negative_times_unsigned() {
    // (-1 signed) * (u64::MAX unsigned)
    // = -1 * (2^64 - 1) = -(2^64 - 1) = -2^64 + 1
    // 128-bit: 0xFFFF_FFFF_FFFF_FFFF_0000_0000_0000_0001
    // Wait, let me compute: (-1i128) * (0xFFFF_FFFF_FFFF_FFFFu128 as i128)
    let a_s = -1i128;
    let b_u = u64::MAX as u128 as i128;
    let expected = ((a_s * b_u) >> 64) as u64;
    assert_eq!(alu(AluOp::Mulhsu, NEG1, U64_MAX, false), expected);
}

#[test]
fn mulhsu_min_times_max() {
    let a_s = i64::MIN as i128;
    let b_u = u64::MAX as u128 as i128;
    let expected = ((a_s * b_u) >> 64) as u64;
    assert_eq!(alu(AluOp::Mulhsu, I64_MIN, U64_MAX, false), expected);
}

#[test]
fn mulhu_zero() {
    assert_eq!(alu(AluOp::Mulhu, ZERO, 42, false), 0);
}

#[test]
fn mulhu_small_values() {
    assert_eq!(alu(AluOp::Mulhu, 100, 200, false), 0);
}

#[test]
fn mulhu_max_times_max() {
    let expected = ((u64::MAX as u128 * u64::MAX as u128) >> 64) as u64;
    assert_eq!(alu(AluOp::Mulhu, U64_MAX, U64_MAX, false), expected);
}

#[test]
fn mulhu_max_times_2() {
    // u64::MAX * 2 = 2^65 - 2. High 64 bits = 1.
    assert_eq!(alu(AluOp::Mulhu, U64_MAX, 2, false), 1);
}

/// RISC-V spec section 7.1: MUL can be checked with MULH for overflow detection.
/// If MULH returns 0 or -1 (sign-extension of MUL result), no overflow occurred.
#[test]
fn mul_mulh_overflow_detection_no_overflow() {
    let a: u64 = 100;
    let b: u64 = 200;
    let lo = alu(AluOp::Mul, a, b, false);
    let hi = alu(AluOp::Mulh, a, b, false);
    // No overflow: hi should be sign-extension of lo's MSB
    let expected_hi = if (lo as i64) < 0 { NEG1 } else { 0 };
    assert_eq!(hi, expected_hi);
}

#[test]
fn mul_mulh_overflow_detection_with_overflow() {
    let a = I64_MAX;
    let b: u64 = 2;
    let lo = alu(AluOp::Mul, a, b, false);
    let hi = alu(AluOp::Mulh, a, b, false);
    // Overflow: hi should NOT be a simple sign-extension of lo
    let sign_ext = if (lo as i64) < 0 { NEG1 } else { 0 };
    assert_ne!(hi, sign_ext, "MULH should indicate overflow occurred");
}

// ═════════════════════════════════════════════════════════════════════════════
//  DIV / DIVW  (Signed Division)
// ═════════════════════════════════════════════════════════════════════════════

/// RISC-V spec section 7.2: Division by zero returns -1 (all bits set).
#[test]
fn div_rv64_divide_by_zero() {
    assert_eq!(alu(AluOp::Div, 42, ZERO, false), NEG1);
}

#[test]
fn div_rv64_zero_divide_by_zero() {
    assert_eq!(alu(AluOp::Div, ZERO, ZERO, false), NEG1);
}

#[test]
fn div_rv64_min_divide_by_zero() {
    assert_eq!(alu(AluOp::Div, I64_MIN, ZERO, false), NEG1);
}

/// RISC-V spec section 7.2: Signed overflow (MIN / -1) returns MIN.
#[test]
fn div_rv64_signed_overflow() {
    assert_eq!(alu(AluOp::Div, I64_MIN, NEG1, false), I64_MIN);
}

#[test]
fn div_rv64_identity() {
    assert_eq!(alu(AluOp::Div, 42, ONE, false), 42);
}

#[test]
fn div_rv64_self_divide() {
    assert_eq!(alu(AluOp::Div, 42, 42, false), 1);
    assert_eq!(alu(AluOp::Div, NEG1, NEG1, false), 1);
}

#[test]
fn div_rv64_positive_by_positive() {
    assert_eq!(alu(AluOp::Div, 100, 7, false), 14); // truncates toward zero
}

#[test]
fn div_rv64_negative_by_positive() {
    // -100 / 7 = -14 (truncated toward zero, not floored)
    assert_eq!(alu(AluOp::Div, -100i64 as u64, 7, false), -14i64 as u64);
}

#[test]
fn div_rv64_positive_by_negative() {
    assert_eq!(alu(AluOp::Div, 100, -7i64 as u64, false), -14i64 as u64);
}

#[test]
fn div_rv64_negative_by_negative() {
    assert_eq!(alu(AluOp::Div, -100i64 as u64, -7i64 as u64, false), 14);
}

#[test]
fn div_rv64_neg1_by_1() {
    assert_eq!(alu(AluOp::Div, NEG1, ONE, false), NEG1);
}

#[test]
fn divw_divide_by_zero() {
    // DIVW: divisor[31:0] == 0 → result = 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(alu(AluOp::Div, 42, ZERO, true), NEG1);
}

/// RISC-V spec: DIVW signed overflow: i32::MIN / -1 = i32::MIN sign-extended.
#[test]
fn divw_signed_overflow() {
    assert_eq!(alu(AluOp::Div, I32_MIN, NEG1, true), I32_MIN);
}

#[test]
fn divw_basic() {
    assert_eq!(alu(AluOp::Div, 100, 7, true), 14);
}

#[test]
fn divw_negative_result_sign_extends() {
    assert_eq!(
        alu(AluOp::Div, -100i64 as u64, 7, true),
        sext32(-14i32 as u32)
    );
}

#[test]
fn divw_ignores_upper_bits() {
    // Upper 32 bits should be ignored for the operation
    assert_eq!(
        alu(
            AluOp::Div,
            0xDEAD_0000_0000_0064,
            0xBEEF_0000_0000_0007,
            true
        ),
        14 // 100 / 7
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  DIVU / DIVUW  (Unsigned Division)
// ═════════════════════════════════════════════════════════════════════════════

/// RISC-V spec section 7.2: Unsigned division by zero returns 2^XLEN - 1.
#[test]
fn divu_rv64_divide_by_zero() {
    assert_eq!(alu(AluOp::Divu, 42, ZERO, false), U64_MAX);
}

#[test]
fn divu_rv64_zero_divide_by_zero() {
    assert_eq!(alu(AluOp::Divu, ZERO, ZERO, false), U64_MAX);
}

#[test]
fn divu_rv64_identity() {
    assert_eq!(alu(AluOp::Divu, 42, ONE, false), 42);
}

#[test]
fn divu_rv64_self_divide() {
    assert_eq!(alu(AluOp::Divu, 42, 42, false), 1);
}

#[test]
fn divu_rv64_large_unsigned() {
    // Treat as unsigned: 0x8000...0 / 2 = 0x4000...0
    assert_eq!(alu(AluOp::Divu, I64_MIN, 2, false), 0x4000_0000_0000_0000);
}

#[test]
fn divu_rv64_max_by_1() {
    assert_eq!(alu(AluOp::Divu, U64_MAX, ONE, false), U64_MAX);
}

#[test]
fn divu_rv64_max_by_max() {
    assert_eq!(alu(AluOp::Divu, U64_MAX, U64_MAX, false), 1);
}

#[test]
fn divu_rv64_basic() {
    assert_eq!(alu(AluOp::Divu, 100, 7, false), 14);
}

#[test]
fn divuw_divide_by_zero() {
    // DIVUW: divisor[31:0] == 0 → result = 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(alu(AluOp::Divu, 42, ZERO, true), NEG1);
}

/// REGRESSION: Phase 0 Bug — DIVU zero-check casts to i32 instead of u32.
/// This test validates that the zero-check is performed on the unsigned
/// 32-bit truncation, not a signed cast. While both happen to be equivalent
/// for the zero check itself, the semantic intent matters.
#[test]
fn divuw_divide_by_zero_upper_bits_set() {
    // b = 0x1_0000_0000 → b[31:0] = 0 → divide by zero
    // This should return 0xFFFF_FFFF_FFFF_FFFF regardless of upper bits.
    let b_with_upper_bits = 0x0000_0001_0000_0000_u64;
    assert_eq!(alu(AluOp::Divu, 42, b_with_upper_bits, true), NEG1);
}

#[test]
fn divuw_basic() {
    assert_eq!(alu(AluOp::Divu, 100, 7, true), 14);
}

#[test]
fn divuw_high_bit_set_is_unsigned() {
    // 0x8000_0000 / 1 = 0x8000_0000 (treated as 2^31 unsigned, not -2^31)
    // Sign-extended result: 0xFFFF_FFFF_8000_0000
    assert_eq!(
        alu(AluOp::Divu, HIGH_BIT_32, ONE, true),
        sext32(0x8000_0000)
    );
}

#[test]
fn divuw_u32_max_by_1() {
    // 0xFFFF_FFFF / 1 = 0xFFFF_FFFF → sign-extended = 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(alu(AluOp::Divu, U32_MAX, ONE, true), NEG1);
}

#[test]
fn divuw_u32_max_by_2() {
    // 0xFFFF_FFFF / 2 = 0x7FFF_FFFF → sign-extended = 0x0000_0000_7FFF_FFFF
    assert_eq!(alu(AluOp::Divu, U32_MAX, 2, true), sext32(0x7FFF_FFFF));
}

#[test]
fn divuw_ignores_upper_input_bits() {
    // a = 0xFFFF_FFFF_0000_0064, b = 0xFFFF_FFFF_0000_0007
    // DIVUW sees a[31:0]=100, b[31:0]=7 → 14
    assert_eq!(
        alu(
            AluOp::Divu,
            0xFFFF_FFFF_0000_0064,
            0xFFFF_FFFF_0000_0007,
            true
        ),
        14
    );
}

/// REGRESSION: Verify DIVUW result is sign-extended from bit 31.
/// The quotient 0x8000_0001 has bit 31 set → must become 0xFFFF_FFFF_8000_0001.
#[test]
fn divuw_result_sign_extends_when_bit31_set() {
    // 0xFFFF_FFFE / 1 = 0xFFFF_FFFE → sign-extended = 0xFFFF_FFFF_FFFF_FFFE
    // But more interesting: pick values where quotient has bit 31 set.
    // 0x8000_0002 / 1 = 0x8000_0002 → sign-extended
    assert_eq!(
        alu(AluOp::Divu, 0x8000_0002, ONE, true),
        sext32(0x8000_0002)
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  REM / REMW  (Signed Remainder)
// ═════════════════════════════════════════════════════════════════════════════

/// RISC-V spec section 7.2: Remainder by zero returns the dividend.
#[test]
fn rem_rv64_remainder_by_zero() {
    assert_eq!(alu(AluOp::Rem, 42, ZERO, false), 42);
}

#[test]
fn rem_rv64_zero_remainder_by_zero() {
    assert_eq!(alu(AluOp::Rem, ZERO, ZERO, false), 0);
}

#[test]
fn rem_rv64_min_remainder_by_zero() {
    assert_eq!(alu(AluOp::Rem, I64_MIN, ZERO, false), I64_MIN);
}

/// RISC-V spec section 7.2: Signed overflow (MIN % -1) returns 0.
#[test]
fn rem_rv64_signed_overflow() {
    assert_eq!(alu(AluOp::Rem, I64_MIN, NEG1, false), 0);
}

#[test]
fn rem_rv64_exact_division() {
    assert_eq!(alu(AluOp::Rem, 42, 7, false), 0);
}

#[test]
fn rem_rv64_positive_remainder() {
    assert_eq!(alu(AluOp::Rem, 100, 7, false), 2);
}

#[test]
fn rem_rv64_negative_dividend() {
    // -100 % 7 = -2 (sign of remainder matches dividend per RISC-V spec)
    assert_eq!(alu(AluOp::Rem, -100i64 as u64, 7, false), -2i64 as u64);
}

#[test]
fn rem_rv64_negative_divisor() {
    // 100 % -7 = 2 (sign follows dividend)
    assert_eq!(alu(AluOp::Rem, 100, -7i64 as u64, false), 2);
}

#[test]
fn rem_rv64_both_negative() {
    // -100 % -7 = -2
    assert_eq!(
        alu(AluOp::Rem, -100i64 as u64, -7i64 as u64, false),
        -2i64 as u64
    );
}

/// RISC-V spec: div * divisor + rem = dividend (algebraic identity).
#[test]
fn rem_rv64_identity_div_mul_rem() {
    let a = 100_u64;
    let b = 7_u64;
    let q = alu(AluOp::Div, a, b, false);
    let r = alu(AluOp::Rem, a, b, false);
    assert_eq!(
        (q as i64).wrapping_mul(b as i64).wrapping_add(r as i64) as u64,
        a,
        "q*b + r must equal a"
    );
}

#[test]
fn remw_remainder_by_zero() {
    // REMW: divisor[31:0] == 0 → result = dividend[31:0] sign-extended
    assert_eq!(alu(AluOp::Rem, 42, ZERO, true), sext32(42));
}

/// REGRESSION: REMW with zero divisor must return sign-extended lower 32 bits,
/// not the full 64-bit value of `a`.
#[test]
fn remw_remainder_by_zero_upper_bits_must_be_ignored() {
    // a = 0xDEAD_BEEF_0000_002A, b = 0
    // Result must be sext(0x0000_002A) = 42, NOT 0xDEAD_BEEF_0000_002A
    let a = 0xDEAD_BEEF_0000_002A_u64;
    assert_eq!(alu(AluOp::Rem, a, ZERO, true), sext32(0x0000_002A));
}

/// REGRESSION: REMW with zero divisor and negative 32-bit dividend.
#[test]
fn remw_remainder_by_zero_negative_dividend() {
    // a[31:0] = 0x8000_0000 → sign-extended = 0xFFFF_FFFF_8000_0000
    let a = 0x0000_0001_8000_0000_u64; // lower 32 = 0x8000_0000
    assert_eq!(alu(AluOp::Rem, a, ZERO, true), sext32(0x8000_0000));
}

#[test]
fn remw_signed_overflow() {
    // i32::MIN % -1 = 0
    assert_eq!(alu(AluOp::Rem, I32_MIN, NEG1, true), 0);
}

#[test]
fn remw_basic() {
    assert_eq!(alu(AluOp::Rem, 100, 7, true), sext32(2));
}

#[test]
fn remw_negative_result_sign_extends() {
    assert_eq!(
        alu(AluOp::Rem, -100i64 as u64, 7, true),
        sext32(-2i32 as u32)
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  REMU / REMUW  (Unsigned Remainder)
// ═════════════════════════════════════════════════════════════════════════════

/// RISC-V spec: Unsigned remainder by zero returns the dividend.
#[test]
fn remu_rv64_remainder_by_zero() {
    assert_eq!(alu(AluOp::Remu, 42, ZERO, false), 42);
}

#[test]
fn remu_rv64_zero_remainder_by_zero() {
    assert_eq!(alu(AluOp::Remu, ZERO, ZERO, false), 0);
}

#[test]
fn remu_rv64_max_remainder_by_zero() {
    assert_eq!(alu(AluOp::Remu, U64_MAX, ZERO, false), U64_MAX);
}

#[test]
fn remu_rv64_exact_division() {
    assert_eq!(alu(AluOp::Remu, 42, 7, false), 0);
}

#[test]
fn remu_rv64_basic() {
    assert_eq!(alu(AluOp::Remu, 100, 7, false), 2);
}

#[test]
fn remu_rv64_large_unsigned() {
    // All values treated as unsigned
    assert_eq!(alu(AluOp::Remu, U64_MAX, 2, false), 1);
}

/// RISC-V spec: divu * divisor + remu = dividend (unsigned identity).
#[test]
fn remu_rv64_identity_divu_mul_remu() {
    let a = 100_u64;
    let b = 7_u64;
    let q = alu(AluOp::Divu, a, b, false);
    let r = alu(AluOp::Remu, a, b, false);
    assert_eq!(q.wrapping_mul(b).wrapping_add(r), a, "q*b + r must equal a");
}

#[test]
fn remuw_remainder_by_zero() {
    // REMUW: divisor[31:0] == 0 → result = dividend[31:0] sign-extended
    assert_eq!(alu(AluOp::Remu, 42, ZERO, true), sext32(42));
}

/// REGRESSION: Phase 0 Bug — REMU zero-check casts to i32 instead of u32.
/// More critically: REMUW division-by-zero must return a[31:0] sign-extended,
/// not the raw 64-bit `a`.
#[test]
fn remuw_remainder_by_zero_upper_bits_must_be_ignored() {
    // a = 0xDEAD_BEEF_0000_002A, b = 0
    // REMUW must return sext(a[31:0]) = sext(0x2A) = 42
    let a = 0xDEAD_BEEF_0000_002A_u64;
    assert_eq!(alu(AluOp::Remu, a, ZERO, true), sext32(0x0000_002A));
}

/// REGRESSION: REMUW zero divisor with upper bits set in b.
#[test]
fn remuw_remainder_by_zero_divisor_upper_bits_set() {
    // b = 0x1_0000_0000 → b[31:0] = 0 → divide by zero
    let b_with_upper_bits = 0x0000_0001_0000_0000_u64;
    assert_eq!(alu(AluOp::Remu, 42, b_with_upper_bits, true), sext32(42));
}

/// REGRESSION: REMUW zero divisor with bit 31 set in dividend.
#[test]
fn remuw_remainder_by_zero_negative_lower32() {
    // a[31:0] = 0x8000_0000 → sign-extended = 0xFFFF_FFFF_8000_0000
    let a = 0x0000_0001_8000_0000_u64;
    assert_eq!(alu(AluOp::Remu, a, ZERO, true), sext32(0x8000_0000));
}

#[test]
fn remuw_basic() {
    assert_eq!(alu(AluOp::Remu, 100, 7, true), sext32(2));
}

#[test]
fn remuw_high_bit_set_is_unsigned() {
    // 0x8000_0001 % 0x8000_0000 = 1 (unsigned)
    assert_eq!(alu(AluOp::Remu, 0x8000_0001, 0x8000_0000, true), sext32(1));
}

#[test]
fn remuw_u32_max_mod_2() {
    // 0xFFFF_FFFF % 2 = 1 → sign-extended = 1
    assert_eq!(alu(AluOp::Remu, U32_MAX, 2, true), sext32(1));
}

#[test]
fn remuw_ignores_upper_bits() {
    assert_eq!(
        alu(
            AluOp::Remu,
            0xFFFF_FFFF_0000_0064,
            0xFFFF_FFFF_0000_0007,
            true
        ),
        sext32(2) // 100 % 7
    );
}

/// REGRESSION: Verify the unsigned identity holds for REMUW.
#[test]
fn remuw_identity_divuw_mul_remuw() {
    let a: u64 = 0x0000_0000_DEAD_BEEF;
    let b: u64 = 0x0000_0000_0000_0007;
    let q = alu(AluOp::Divu, a, b, true);
    let r = alu(AluOp::Remu, a, b, true);
    // q and r are sign-extended 32-bit results; extract lower 32 bits for check
    let q32 = q as u32;
    let r32 = r as u32;
    let a32 = a as u32;
    assert_eq!(
        q32.wrapping_mul(b as u32).wrapping_add(r32),
        a32,
        "DIVUW*b + REMUW must equal a[31:0]"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  CROSS-CUTTING: Alternating bit patterns & power-of-two boundaries
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn add_rv64_alternating_bits() {
    // 0xAAAA... + 0x5555... = 0xFFFF...
    assert_eq!(
        alu(AluOp::Add, ALTERNATING_A, ALTERNATING_5, false),
        U64_MAX
    );
}

#[test]
fn sub_rv64_alternating_bits() {
    assert_eq!(alu(AluOp::Sub, ALTERNATING_A, ALTERNATING_A, false), 0);
}

#[test]
fn mul_rv64_power_of_two() {
    // x * 2^n should equal x << n (for non-overflowing cases)
    assert_eq!(alu(AluOp::Mul, 0x1234, 1 << 16, false), 0x1234_0000);
}

#[test]
fn div_rv64_power_of_two() {
    // 256 / 16 = 16
    assert_eq!(alu(AluOp::Div, 256, 16, false), 16);
}

#[test]
fn divu_rv64_max_by_power_of_two() {
    assert_eq!(alu(AluOp::Divu, U64_MAX, 1 << 32, false), 0xFFFF_FFFF);
}

#[test]
fn rem_rv64_power_of_two() {
    // 100 % 64 = 36
    assert_eq!(alu(AluOp::Rem, 100, 64, false), 36);
}

#[test]
fn remu_rv64_power_of_two() {
    assert_eq!(alu(AluOp::Remu, 100, 64, false), 36);
}

// ═════════════════════════════════════════════════════════════════════════════
//  CROSS-CUTTING: Verify *W operations always produce sign-extended results
// ═════════════════════════════════════════════════════════════════════════════

/// Every *W operation must produce a value where bits [63:32] are all copies
/// of bit 31. This property must hold regardless of input.
#[test]
fn all_w_operations_produce_sign_extended_results() {
    let test_cases: Vec<(AluOp, u64, u64)> = vec![
        (AluOp::Add, 0xFFFF_FFFF, 1), // wraps to 0
        (AluOp::Add, 0x7FFF_FFFF, 1), // wraps to 0x8000_0000 (negative)
        (AluOp::Sub, 0, 1),           // -1
        (AluOp::Sub, 0x8000_0000, 1), // i32::MAX
        (AluOp::Mul, 0x7FFF_FFFF, 2), // overflow
        (AluOp::Div, 100, 7),
        (AluOp::Div, 0x8000_0000, NEG1), // overflow: MIN/-1
        (AluOp::Divu, 0xFFFF_FFFF, 1),
        (AluOp::Divu, 42, 0), // div by zero
        (AluOp::Rem, 100, 7),
        (AluOp::Rem, 42, 0), // rem by zero
        (AluOp::Remu, 100, 7),
        (AluOp::Remu, 42, 0), // rem by zero
    ];

    for (op, a, b) in test_cases {
        let result = alu(op, a, b, true);
        let bit31 = (result >> 31) & 1;
        let upper = result >> 32;
        let expected_upper = if bit31 == 1 { 0xFFFF_FFFF } else { 0 };
        assert_eq!(
            upper, expected_upper,
            "Op {:?} with a={:#x}, b={:#x}: result {:#018x} is not properly sign-extended",
            op, a, b, result
        );
    }
}

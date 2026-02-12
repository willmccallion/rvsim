//! Atomic ALU Unit Tests.
//!
//! Direct unit tests for the `atomic_alu` function which implements
//! the read-modify-write arithmetic for RISC-V A-extension AMO instructions.
//!
//! All 9 operations (Swap, Add, Xor, And, Or, Min, Max, Minu, Maxu) are tested
//! for both Word (32-bit) and Double (64-bit) widths with edge cases:
//!   - Word sign-extension from bit 31
//!   - Overflow / wrapping behavior
//!   - Signed vs unsigned comparisons with negative values
//!   - Edge values: 0, MAX, MIN for both widths
//!
//! Reference: RISC-V ISA Spec Volume I, Chapter 8 (A Extension).

use riscv_core::core::pipeline::signals::{AtomicOp, MemWidth};
use riscv_core::core::units::lsu::atomic::atomic_alu;

// ─── Constants ───────────────────────────────────────────

// 64-bit boundaries
const I64_MAX: u64 = i64::MAX as u64; // 0x7FFF_FFFF_FFFF_FFFF
const I64_MIN: u64 = i64::MIN as u64; // 0x8000_0000_0000_0000
const U64_MAX: u64 = u64::MAX; // 0xFFFF_FFFF_FFFF_FFFF

// 32-bit boundaries (as 64-bit values, zero-extended)
const I32_MAX: u64 = i32::MAX as u64; // 0x0000_0000_7FFF_FFFF
const U32_MAX: u64 = u32::MAX as u64; // 0x0000_0000_FFFF_FFFF
const I32_MIN_ZEXT: u64 = 0x8000_0000u64; // 0x0000_0000_8000_0000

/// Helper: sign-extend a 32-bit value to 64 bits.
fn sext32(val: u32) -> u64 {
    val as i32 as i64 as u64
}

// ══════════════════════════════════════════════════════════
// 1. Swap
// ══════════════════════════════════════════════════════════

#[test]
fn swap_word_returns_reg_val_sign_extended() {
    // AMOSWAP.W: result = reg_val (as i32, sign-extended to 64 bits)
    assert_eq!(
        atomic_alu(AtomicOp::Swap, 0xDEAD, 42, MemWidth::Word),
        sext32(42)
    );
    // Negative reg_val (bit 31 set)
    assert_eq!(
        atomic_alu(AtomicOp::Swap, 0, 0x8000_0000, MemWidth::Word),
        sext32(0x8000_0000u32)
    );
}

#[test]
fn swap_double_returns_reg_val() {
    assert_eq!(
        atomic_alu(
            AtomicOp::Swap,
            0xDEAD,
            0x1234_5678_9ABC_DEF0,
            MemWidth::Double
        ),
        0x1234_5678_9ABC_DEF0
    );
}

// ══════════════════════════════════════════════════════════
// 2. Add
// ══════════════════════════════════════════════════════════

#[test]
fn add_word_basic() {
    assert_eq!(
        atomic_alu(AtomicOp::Add, 10, 20, MemWidth::Word),
        sext32(30)
    );
}

#[test]
fn add_word_wrapping_overflow() {
    // 0x7FFF_FFFF + 1 = 0x8000_0000 (wraps into negative 32-bit)
    let result = atomic_alu(AtomicOp::Add, I32_MAX, 1, MemWidth::Word);
    assert_eq!(result, sext32(0x8000_0000u32));
    // Verify sign-extension: bit 63 should be set
    assert_ne!(result & (1 << 63), 0);
}

#[test]
fn add_word_wrapping_underflow() {
    // 0x8000_0000 + (-1) = 0x7FFF_FFFF (wraps into positive)
    let result = atomic_alu(AtomicOp::Add, I32_MIN_ZEXT, U32_MAX, MemWidth::Word);
    // i32: -2147483648 + (-1) = wrapping to 0x7FFF_FFFF
    assert_eq!(result, sext32(0x7FFF_FFFFu32));
}

#[test]
fn add_word_zero() {
    assert_eq!(atomic_alu(AtomicOp::Add, 42, 0, MemWidth::Word), sext32(42));
}

#[test]
fn add_double_basic() {
    assert_eq!(atomic_alu(AtomicOp::Add, 100, 200, MemWidth::Double), 300);
}

#[test]
fn add_double_wrapping_overflow() {
    let result = atomic_alu(AtomicOp::Add, I64_MAX, 1, MemWidth::Double);
    assert_eq!(result, I64_MIN); // wraps
}

#[test]
fn add_double_neg1() {
    assert_eq!(atomic_alu(AtomicOp::Add, 1, U64_MAX, MemWidth::Double), 0);
}

// ══════════════════════════════════════════════════════════
// 3. Xor
// ══════════════════════════════════════════════════════════

#[test]
fn xor_word() {
    assert_eq!(
        atomic_alu(AtomicOp::Xor, 0xFF00_FF00, 0x0F0F_0F0F, MemWidth::Word),
        sext32(0xF00F_F00Fu32)
    );
}

#[test]
fn xor_double() {
    assert_eq!(
        atomic_alu(
            AtomicOp::Xor,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
            MemWidth::Double
        ),
        U64_MAX
    );
}

#[test]
fn xor_self_is_zero() {
    assert_eq!(
        atomic_alu(AtomicOp::Xor, 0x1234_5678, 0x1234_5678, MemWidth::Word),
        sext32(0)
    );
}

// ══════════════════════════════════════════════════════════
// 4. And
// ══════════════════════════════════════════════════════════

#[test]
fn and_word() {
    assert_eq!(
        atomic_alu(AtomicOp::And, 0xFF00_FF00, 0x0F0F_0F0F, MemWidth::Word),
        sext32(0x0F00_0F00u32)
    );
}

#[test]
fn and_double() {
    assert_eq!(
        atomic_alu(
            AtomicOp::And,
            U64_MAX,
            0x0000_FFFF_0000_FFFF,
            MemWidth::Double
        ),
        0x0000_FFFF_0000_FFFF
    );
}

#[test]
fn and_with_zero() {
    assert_eq!(atomic_alu(AtomicOp::And, U64_MAX, 0, MemWidth::Double), 0);
}

// ══════════════════════════════════════════════════════════
// 5. Or
// ══════════════════════════════════════════════════════════

#[test]
fn or_word() {
    assert_eq!(
        atomic_alu(AtomicOp::Or, 0xF000_0000, 0x000F_0000, MemWidth::Word),
        sext32(0xF00F_0000u32)
    );
}

#[test]
fn or_double() {
    assert_eq!(
        atomic_alu(
            AtomicOp::Or,
            0xAAAA_0000_0000_0000,
            0x0000_0000_0000_5555,
            MemWidth::Double
        ),
        0xAAAA_0000_0000_5555
    );
}

#[test]
fn or_with_zero() {
    assert_eq!(atomic_alu(AtomicOp::Or, 42, 0, MemWidth::Double), 42);
}

// ══════════════════════════════════════════════════════════
// 6. Min (signed)
// ══════════════════════════════════════════════════════════

#[test]
fn min_word_positive() {
    assert_eq!(
        atomic_alu(AtomicOp::Min, 10, 20, MemWidth::Word),
        sext32(10)
    );
}

#[test]
fn min_word_negative_values() {
    // -1 vs -2 → min is -2
    let neg1 = (-1i32 as u32) as u64;
    let neg2 = (-2i32 as u32) as u64;
    assert_eq!(
        atomic_alu(AtomicOp::Min, neg1, neg2, MemWidth::Word),
        sext32(-2i32 as u32)
    );
}

#[test]
fn min_word_mixed_sign() {
    // 1 vs -1 → min is -1 (signed comparison)
    let neg1 = (-1i32 as u32) as u64;
    assert_eq!(
        atomic_alu(AtomicOp::Min, 1, neg1, MemWidth::Word),
        sext32(-1i32 as u32)
    );
}

#[test]
fn min_double_negative() {
    let neg1 = (-1i64) as u64;
    let neg100 = (-100i64) as u64;
    assert_eq!(
        atomic_alu(AtomicOp::Min, neg1, neg100, MemWidth::Double),
        neg100
    );
}

#[test]
fn min_word_edge_i32_min_max() {
    assert_eq!(
        atomic_alu(AtomicOp::Min, I32_MAX, I32_MIN_ZEXT, MemWidth::Word),
        sext32(i32::MIN as u32)
    );
}

#[test]
fn min_double_edge_i64_min_max() {
    assert_eq!(
        atomic_alu(AtomicOp::Min, I64_MAX, I64_MIN, MemWidth::Double),
        I64_MIN
    );
}

// ══════════════════════════════════════════════════════════
// 7. Max (signed)
// ══════════════════════════════════════════════════════════

#[test]
fn max_word_positive() {
    assert_eq!(
        atomic_alu(AtomicOp::Max, 10, 20, MemWidth::Word),
        sext32(20)
    );
}

#[test]
fn max_word_negative_values() {
    let neg1 = (-1i32 as u32) as u64;
    let neg2 = (-2i32 as u32) as u64;
    assert_eq!(
        atomic_alu(AtomicOp::Max, neg1, neg2, MemWidth::Word),
        sext32(-1i32 as u32)
    );
}

#[test]
fn max_word_mixed_sign() {
    let neg1 = (-1i32 as u32) as u64;
    assert_eq!(
        atomic_alu(AtomicOp::Max, 1, neg1, MemWidth::Word),
        sext32(1)
    );
}

#[test]
fn max_double_negative() {
    let neg1 = (-1i64) as u64;
    let neg100 = (-100i64) as u64;
    assert_eq!(
        atomic_alu(AtomicOp::Max, neg1, neg100, MemWidth::Double),
        neg1
    );
}

#[test]
fn max_double_edge_i64_min_max() {
    assert_eq!(
        atomic_alu(AtomicOp::Max, I64_MAX, I64_MIN, MemWidth::Double),
        I64_MAX
    );
}

// ══════════════════════════════════════════════════════════
// 8. Minu (unsigned)
// ══════════════════════════════════════════════════════════

#[test]
fn minu_word_basic() {
    assert_eq!(
        atomic_alu(AtomicOp::Minu, 10, 20, MemWidth::Word),
        sext32(10)
    );
}

#[test]
fn minu_word_large_unsigned() {
    // 0xFFFF_FFFF vs 0x0000_0001 → unsigned min is 1
    assert_eq!(
        atomic_alu(AtomicOp::Minu, U32_MAX, 1, MemWidth::Word),
        sext32(1)
    );
}

#[test]
fn minu_word_high_bit_set_is_large() {
    // Unsigned: 0x8000_0000 > 0x7FFF_FFFF (unlike signed)
    assert_eq!(
        atomic_alu(AtomicOp::Minu, I32_MIN_ZEXT, I32_MAX, MemWidth::Word),
        sext32(I32_MAX as u32)
    );
}

#[test]
fn minu_double_basic() {
    assert_eq!(atomic_alu(AtomicOp::Minu, 100, 200, MemWidth::Double), 100);
}

#[test]
fn minu_double_large_unsigned() {
    // 0xFFFF...FFFF vs 1 → min is 1
    assert_eq!(atomic_alu(AtomicOp::Minu, U64_MAX, 1, MemWidth::Double), 1);
}

#[test]
fn minu_double_zero_is_minimum() {
    assert_eq!(atomic_alu(AtomicOp::Minu, 0, U64_MAX, MemWidth::Double), 0);
}

// ══════════════════════════════════════════════════════════
// 9. Maxu (unsigned)
// ══════════════════════════════════════════════════════════

#[test]
fn maxu_word_basic() {
    assert_eq!(
        atomic_alu(AtomicOp::Maxu, 10, 20, MemWidth::Word),
        sext32(20)
    );
}

#[test]
fn maxu_word_large_unsigned() {
    // Unsigned: 0xFFFF_FFFF is max
    assert_eq!(
        atomic_alu(AtomicOp::Maxu, U32_MAX, 1, MemWidth::Word),
        sext32(U32_MAX as u32)
    );
}

#[test]
fn maxu_word_high_bit_set_is_large() {
    // Unsigned: 0x8000_0000 > 0x7FFF_FFFF
    assert_eq!(
        atomic_alu(AtomicOp::Maxu, I32_MIN_ZEXT, I32_MAX, MemWidth::Word),
        sext32(0x8000_0000u32)
    );
}

#[test]
fn maxu_double_large_unsigned() {
    assert_eq!(
        atomic_alu(AtomicOp::Maxu, U64_MAX, 1, MemWidth::Double),
        U64_MAX
    );
}

#[test]
fn maxu_double_zero_and_max() {
    assert_eq!(
        atomic_alu(AtomicOp::Maxu, 0, U64_MAX, MemWidth::Double),
        U64_MAX
    );
}

// ══════════════════════════════════════════════════════════
// 10. Word Sign-Extension Verification
// ══════════════════════════════════════════════════════════

/// All Word-width operations must produce results sign-extended from bit 31.
/// This test spot-checks multiple ops.
#[test]
fn word_sign_extension_positive_result() {
    // Add: 1 + 1 = 2 (positive, upper 32 bits should be 0).
    let result = atomic_alu(AtomicOp::Add, 1, 1, MemWidth::Word);
    assert_eq!(result, 2);
    assert_eq!(
        result >> 32,
        0,
        "Upper 32 bits should be 0 for positive result"
    );
}

#[test]
fn word_sign_extension_negative_result() {
    // Add: 0 + 0xFFFF_FFFF = -1 as i32. Sign-extended to 0xFFFF_FFFF_FFFF_FFFF.
    let result = atomic_alu(AtomicOp::Add, 0, U32_MAX, MemWidth::Word);
    assert_eq!(
        result, U64_MAX,
        "0 + (-1 as i32) should sign-extend to all-ones"
    );
}

#[test]
fn word_sign_extension_from_xor() {
    // Xor: 0x7000_0000 ^ 0xF000_0000 = 0x8000_0000 → sign-extends to 0xFFFF_FFFF_8000_0000.
    let result = atomic_alu(AtomicOp::Xor, 0x7000_0000, 0xF000_0000, MemWidth::Word);
    assert_eq!(result, sext32(0x8000_0000u32));
    assert_eq!(result >> 32, 0xFFFF_FFFF);
}

#[test]
fn word_sign_extension_from_or() {
    // Or: 0x8000_0000 | 0 = 0x8000_0000 → negative → sign-extends.
    let result = atomic_alu(AtomicOp::Or, 0x8000_0000, 0, MemWidth::Word);
    assert_eq!(result, sext32(0x8000_0000u32));
}

// ══════════════════════════════════════════════════════════
// 11. Edge Values (0, MAX, MIN)
// ══════════════════════════════════════════════════════════

#[test]
fn add_word_zero_plus_zero() {
    assert_eq!(atomic_alu(AtomicOp::Add, 0, 0, MemWidth::Word), 0);
}

#[test]
fn add_double_zero_plus_zero() {
    assert_eq!(atomic_alu(AtomicOp::Add, 0, 0, MemWidth::Double), 0);
}

#[test]
fn swap_word_zero() {
    assert_eq!(atomic_alu(AtomicOp::Swap, U64_MAX, 0, MemWidth::Word), 0);
}

#[test]
fn swap_double_zero() {
    assert_eq!(atomic_alu(AtomicOp::Swap, U64_MAX, 0, MemWidth::Double), 0);
}

#[test]
fn min_word_equal_values() {
    assert_eq!(
        atomic_alu(AtomicOp::Min, 42, 42, MemWidth::Word),
        sext32(42)
    );
}

#[test]
fn max_word_equal_values() {
    assert_eq!(
        atomic_alu(AtomicOp::Max, 42, 42, MemWidth::Word),
        sext32(42)
    );
}

#[test]
fn minu_word_equal_values() {
    assert_eq!(
        atomic_alu(AtomicOp::Minu, 42, 42, MemWidth::Word),
        sext32(42)
    );
}

#[test]
fn maxu_word_equal_values() {
    assert_eq!(
        atomic_alu(AtomicOp::Maxu, 42, 42, MemWidth::Word),
        sext32(42)
    );
}

#[test]
fn and_word_all_ones() {
    assert_eq!(
        atomic_alu(AtomicOp::And, U32_MAX, U32_MAX, MemWidth::Word),
        sext32(U32_MAX as u32)
    );
}

#[test]
fn or_double_zero_or_zero() {
    assert_eq!(atomic_alu(AtomicOp::Or, 0, 0, MemWidth::Double), 0);
}

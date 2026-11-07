//! ALU arithmetic operations.
//!
//! Implements integer addition, subtraction, multiplication, and division
//! for both RV64 (64-bit) and RV32 (32-bit word) variants. Includes the
//! full M-extension multiply/divide family.
//!
//! All 32-bit (`IntWidth::Word`) results are sign-extended from bit 31 to
//! 64 bits, per the RISC-V spec (§2.4, §7.1, §7.2).

use crate::isa::op::{AluOp, IntWidth};

const WORD_BITS: u32 = 32;

const XLEN_BITS: u32 = 64;

/// Executes an integer arithmetic operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be an arithmetic variant).
/// * `a`    - First operand (64-bit value).
/// * `b`    - Second operand (64-bit value).
/// * `width` - `Word` for the 32-bit (W-suffix) variant.
///
/// # Returns
///
/// The 64-bit result. For 32-bit operations the result is sign-extended
/// from bit 31. Returns `0` for non-arithmetic opcodes.
pub const fn execute(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        AluOp::Add => match width {
            IntWidth::Word => (a as i32).wrapping_add(b as i32) as i64 as u64,
            IntWidth::Double => a.wrapping_add(b),
        },
        AluOp::Sub => match width {
            IntWidth::Word => (a as i32).wrapping_sub(b as i32) as i64 as u64,
            IntWidth::Double => a.wrapping_sub(b),
        },
        AluOp::Mul => match width {
            IntWidth::Word => (a as i32).wrapping_mul(b as i32) as i64 as u64,
            IntWidth::Double => a.wrapping_mul(b),
        },
        AluOp::Mulh => {
            match width {
                IntWidth::Word => ((a as i32 as i64 * b as i32 as i64) >> WORD_BITS) as u64,
                IntWidth::Double => {
                    // Both operands are signed: sign-extend through i64 to preserve
                    // negative values. Direct u64→i128 zero-extends (RISC-V spec §7.1).
                    (((a as i64 as i128) * (b as i64 as i128)) >> XLEN_BITS) as u64
                }
            }
        }
        AluOp::Mulhsu => {
            match width {
                IntWidth::Word => ((a as i32 as i64 * (b as u32) as i64) >> WORD_BITS) as u64,
                IntWidth::Double => {
                    // Operand a is signed, b is unsigned. Sign-extend a through
                    // i64 to i128; zero-extend b through u128 (RISC-V spec §7.1).
                    (((a as i64 as i128) * (b as u128 as i128)) >> XLEN_BITS) as u64
                }
            }
        }
        AluOp::Mulhu => match width {
            IntWidth::Word => (((a as u32) as u64 * (b as u32) as u64) >> WORD_BITS) as i64 as u64,
            IntWidth::Double => (((a as u128) * (b as u128)) >> XLEN_BITS) as u64,
        },
        AluOp::Div => match width {
            IntWidth::Word => {
                if (b as i32) == 0 {
                    -1i64 as u64
                } else {
                    (a as i32).wrapping_div(b as i32) as i64 as u64
                }
            }
            IntWidth::Double => {
                if b == 0 {
                    -1i64 as u64
                } else {
                    (a as i64).wrapping_div(b as i64) as u64
                }
            }
        },
        AluOp::Divu => match width {
            IntWidth::Word => match (a as u32).checked_div(b as u32) {
                Some(q) => q as i32 as i64 as u64,
                None => -1i64 as u64,
            },
            IntWidth::Double => match a.checked_div(b) {
                Some(q) => q,
                None => -1i64 as u64,
            },
        },
        AluOp::Rem => {
            match width {
                IntWidth::Word => {
                    if (b as i32) == 0 {
                        // REMW div-by-zero: return dividend[31:0] sign-extended
                        // to 64 bits, not the raw 64-bit a (RISC-V spec §7.2).
                        (a as i32) as i64 as u64
                    } else {
                        (a as i32).wrapping_rem(b as i32) as i64 as u64
                    }
                }
                IntWidth::Double => {
                    if b == 0 {
                        a
                    } else {
                        (a as i64).wrapping_rem(b as i64) as u64
                    }
                }
            }
        }
        AluOp::Remu => {
            match width {
                IntWidth::Word => {
                    if (b as u32) == 0 {
                        // REMUW div-by-zero: return dividend[31:0] sign-extended
                        // to 64 bits (RISC-V spec §7.2).
                        (a as u32) as i32 as i64 as u64
                    } else {
                        ((a as u32) % (b as u32)) as i32 as i64 as u64
                    }
                }
                IntWidth::Double => {
                    if b == 0 {
                        a
                    } else {
                        a % b
                    }
                }
            }
        }
        _ => 0,
    }
}

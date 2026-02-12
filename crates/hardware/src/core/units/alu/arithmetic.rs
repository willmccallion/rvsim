//! ALU arithmetic operations.
//!
//! Implements integer addition, subtraction, multiplication, and division
//! for both RV64 (64-bit) and RV32 (32-bit word) variants. Includes the
//! full M-extension multiply/divide family.
//!
//! All 32-bit (`is32 == true`) results are sign-extended from bit 31 to
//! 64 bits, per the RISC-V spec (§2.4, §7.1, §7.2).

use crate::core::pipeline::signals::AluOp;

/// Number of bits in a 32-bit word (used for high-multiply shift).
const WORD_BITS: u32 = 32;

/// Number of bits in XLEN for RV64 (used for high-multiply shift).
const XLEN_BITS: u32 = 64;

/// Executes an integer arithmetic operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be an arithmetic variant).
/// * `a`    - First operand (64-bit value).
/// * `b`    - Second operand (64-bit value).
/// * `is32` - If true, perform the 32-bit (W-suffix) variant.
///
/// # Returns
///
/// The 64-bit result. For 32-bit operations the result is sign-extended
/// from bit 31. Returns `0` for non-arithmetic opcodes.
pub fn execute(op: AluOp, a: u64, b: u64, is32: bool) -> u64 {
    match op {
        AluOp::Add => {
            if is32 {
                (a as i32).wrapping_add(b as i32) as i64 as u64
            } else {
                a.wrapping_add(b)
            }
        }
        AluOp::Sub => {
            if is32 {
                (a as i32).wrapping_sub(b as i32) as i64 as u64
            } else {
                a.wrapping_sub(b)
            }
        }
        AluOp::Mul => {
            if is32 {
                (a as i32).wrapping_mul(b as i32) as i64 as u64
            } else {
                a.wrapping_mul(b)
            }
        }
        AluOp::Mulh => {
            if is32 {
                ((a as i32 as i64 * b as i32 as i64) >> WORD_BITS) as u64
            } else {
                // Both operands are signed: sign-extend through i64 to preserve
                // negative values. Direct u64→i128 zero-extends (RISC-V spec §7.1).
                (((a as i64 as i128) * (b as i64 as i128)) >> XLEN_BITS) as u64
            }
        }
        AluOp::Mulhsu => {
            if is32 {
                ((a as i32 as i64 * (b as u32) as i64) >> WORD_BITS) as u64
            } else {
                // Operand a is signed, b is unsigned. Sign-extend a through
                // i64 to i128; zero-extend b through u128 (RISC-V spec §7.1).
                (((a as i64 as i128) * (b as u128 as i128)) >> XLEN_BITS) as u64
            }
        }
        AluOp::Mulhu => {
            if is32 {
                (((a as u32) as u64 * (b as u32) as u64) >> WORD_BITS) as i64 as u64
            } else {
                (((a as u128) * (b as u128)) >> XLEN_BITS) as u64
            }
        }
        AluOp::Div => {
            if is32 {
                if (b as i32) == 0 {
                    -1i64 as u64
                } else {
                    (a as i32).wrapping_div(b as i32) as i64 as u64
                }
            } else if b == 0 {
                -1i64 as u64
            } else {
                (a as i64).wrapping_div(b as i64) as u64
            }
        }
        AluOp::Divu => {
            if is32 {
                // Phase 0 fix: use u32 cast for unsigned zero-check, and
                // sign-extend result from bit 31 via i32 (RISC-V spec §7.2).
                if (b as u32) == 0 {
                    -1i64 as u64
                } else {
                    ((a as u32) / (b as u32)) as i32 as i64 as u64
                }
            } else if b == 0 {
                -1i64 as u64
            } else {
                a / b
            }
        }
        AluOp::Rem => {
            if is32 {
                if (b as i32) == 0 {
                    // REMW div-by-zero: return dividend[31:0] sign-extended
                    // to 64 bits, not the raw 64-bit a (RISC-V spec §7.2).
                    (a as i32) as i64 as u64
                } else {
                    (a as i32).wrapping_rem(b as i32) as i64 as u64
                }
            } else if b == 0 {
                a
            } else {
                (a as i64).wrapping_rem(b as i64) as u64
            }
        }
        AluOp::Remu => {
            if is32 {
                // Phase 0 fix: use u32 cast for unsigned zero-check.
                if (b as u32) == 0 {
                    // REMUW div-by-zero: return dividend[31:0] sign-extended
                    // to 64 bits (RISC-V spec §7.2).
                    (a as u32) as i32 as i64 as u64
                } else {
                    ((a as u32) % (b as u32)) as i32 as i64 as u64
                }
            } else if b == 0 {
                a
            } else {
                a % b
            }
        }
        _ => 0,
    }
}

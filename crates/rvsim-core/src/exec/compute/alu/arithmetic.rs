//! ALU arithmetic operations.
//!
//! Implements integer addition, subtraction, multiplication, and division
//! for both RV64 (64-bit) and RV32 (32-bit word) variants. Includes the
//! full M-extension multiply/divide family.
//!
//! All 32-bit (`IntWidth::Word`) results are sign-extended from bit 31 to
//! 64 bits, per the RISC-V spec (§2.4, §7.1, §7.2).

use crate::isa::bits::sext_word;
use crate::isa::op::{ArithOp, DivOp, IntWidth, MulOp};

const WORD_BITS: u32 = 32;

const XLEN_BITS: u32 = 64;

/// `add` or `sub`.
pub const fn add_sub(op: ArithOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        ArithOp::Add => match width {
            IntWidth::Word => ((a as i32).wrapping_add(b as i32) as i64).cast_unsigned(),
            IntWidth::Double => a.wrapping_add(b),
        },
        ArithOp::Sub => match width {
            IntWidth::Word => ((a as i32).wrapping_sub(b as i32) as i64).cast_unsigned(),
            IntWidth::Double => a.wrapping_sub(b),
        },
    }
}

/// A multiplication: the low or a high half of the product.
pub const fn multiply(op: MulOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        MulOp::Mul => match width {
            IntWidth::Word => ((a as i32).wrapping_mul(b as i32) as i64).cast_unsigned(),
            IntWidth::Double => a.wrapping_mul(b),
        },
        MulOp::Mulh => {
            match width {
                IntWidth::Word => {
                    ((a as i32 as i64 * b as i32 as i64) >> WORD_BITS).cast_unsigned()
                }
                IntWidth::Double => {
                    // Both operands are signed: sign-extend through i64 to preserve
                    // negative values. Direct u64→i128 zero-extends (RISC-V spec §7.1).
                    (((a.cast_signed() as i128) * (b.cast_signed() as i128)) >> XLEN_BITS) as u64
                }
            }
        }
        MulOp::Mulhsu => {
            match width {
                IntWidth::Word => {
                    ((a as i32 as i64 * (b as u32) as i64) >> WORD_BITS).cast_unsigned()
                }
                IntWidth::Double => {
                    // Operand a is signed, b is unsigned. Sign-extend a through
                    // i64 to i128; zero-extend b through u128 (RISC-V spec §7.1).
                    (((a.cast_signed() as i128) * (b as u128).cast_signed()) >> XLEN_BITS) as u64
                }
            }
        }
        MulOp::Mulhu => match width {
            IntWidth::Word => {
                (((a as u32) as u64 * (b as u32) as u64) >> WORD_BITS).cast_signed().cast_unsigned()
            }
            IntWidth::Double => (((a as u128) * (b as u128)) >> XLEN_BITS) as u64,
        },
    }
}

/// A division or remainder, with RISC-V's results for a zero divisor and
/// for overflow.
pub const fn divide(op: DivOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        DivOp::Div => match width {
            IntWidth::Word => {
                if (b as i32) == 0 {
                    (-1i64).cast_unsigned()
                } else {
                    ((a as i32).wrapping_div(b as i32) as i64).cast_unsigned()
                }
            }
            IntWidth::Double => {
                if b == 0 {
                    (-1i64).cast_unsigned()
                } else {
                    a.cast_signed().wrapping_div(b.cast_signed()).cast_unsigned()
                }
            }
        },
        DivOp::Divu => match width {
            IntWidth::Word => match (a as u32).checked_div(b as u32) {
                Some(q) => sext_word(q),
                None => (-1i64).cast_unsigned(),
            },
            IntWidth::Double => match a.checked_div(b) {
                Some(q) => q,
                None => (-1i64).cast_unsigned(),
            },
        },
        DivOp::Rem => {
            match width {
                IntWidth::Word => {
                    if (b as i32) == 0 {
                        // REMW div-by-zero: return dividend[31:0] sign-extended
                        // to 64 bits, not the raw 64-bit a (RISC-V spec §7.2).
                        ((a as i32) as i64).cast_unsigned()
                    } else {
                        ((a as i32).wrapping_rem(b as i32) as i64).cast_unsigned()
                    }
                }
                IntWidth::Double => {
                    if b == 0 {
                        a
                    } else {
                        a.cast_signed().wrapping_rem(b.cast_signed()).cast_unsigned()
                    }
                }
            }
        }
        DivOp::Remu => {
            match width {
                IntWidth::Word => {
                    if (b as u32) == 0 {
                        // REMUW div-by-zero: return dividend[31:0] sign-extended
                        // to 64 bits (RISC-V spec §7.2).
                        sext_word(a as u32)
                    } else {
                        sext_word((a as u32) % (b as u32))
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
    }
}

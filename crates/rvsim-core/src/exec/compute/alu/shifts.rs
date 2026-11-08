//! ALU shift operations.
//!
//! Implements shift-left logical (SLL), shift-right logical (SRL), and
//! shift-right arithmetic (SRA) for both RV64 and RV32 variants.
//!
//! Shift amounts are masked to 6 bits (0–63) for RV64 and 5 bits (0–31)
//! for RV32, per RISC-V spec §2.4. All 32-bit results are sign-extended
//! from bit 31 to 64 bits.

use crate::isa::bits::sext_word;
use crate::isa::op::{IntWidth, ShiftOp};

const SHAMT_MASK_RV64: u64 = 0x3f;

const SHAMT_MASK_RV32: u32 = 0x1f;

/// Executes a shift operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be a shift variant).
/// * `a`    - The value to be shifted (64-bit).
/// * `b`    - The shift amount (lower bits used, upper bits ignored).
/// * `width` - `Word` for the 32-bit (W-suffix) variant.
///
/// # Returns
///
/// The 64-bit result. For 32-bit operations the result is sign-extended
/// from bit 31.
pub const fn execute(op: ShiftOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let sh6 = (b & SHAMT_MASK_RV64) as u32;
    match op {
        ShiftOp::Sll => match width {
            IntWidth::Word => (a as i32).wrapping_shl(b as u32 & SHAMT_MASK_RV32) as i64 as u64,
            IntWidth::Double => a.wrapping_shl(sh6),
        },
        ShiftOp::Srl => match width {
            IntWidth::Word => sext_word((a as u32).wrapping_shr(b as u32 & SHAMT_MASK_RV32)),
            IntWidth::Double => a.wrapping_shr(sh6),
        },
        ShiftOp::Sra => match width {
            IntWidth::Word => ((a as i32) >> (b as u32 & SHAMT_MASK_RV32)) as i64 as u64,
            IntWidth::Double => ((a as i64) >> sh6) as u64,
        },
    }
}

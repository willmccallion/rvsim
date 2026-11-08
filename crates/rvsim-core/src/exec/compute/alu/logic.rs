//! ALU logical and comparison operations.
//!
//! Implements bitwise OR, AND, XOR, and set-less-than (signed and unsigned)
//! for both RV64 and RV32 variants.
//!
//! For 32-bit comparisons (`Slt`/`Sltu` at `IntWidth::Word`), only the lower 32 bits
//! of each operand are considered. The result is always 0 or 1.

use crate::isa::op::{IntWidth, LogicOp};

/// Executes a logical or comparison operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be a logic/comparison variant).
/// * `a`    - First operand (64-bit value).
/// * `b`    - Second operand (64-bit value).
/// * `width` - `Word` for the 32-bit comparison variant of Slt/Sltu.
///
/// # Returns
///
/// The 64-bit result. Bitwise operations always use the full 64 bits
/// regardless of `width`.
pub const fn execute(op: LogicOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        LogicOp::Or => a | b,
        LogicOp::And => a & b,
        LogicOp::Xor => a ^ b,
        LogicOp::Slt => match width {
            IntWidth::Word => ((a as i32) < (b as i32)) as u64,
            IntWidth::Double => (a.cast_signed() < b.cast_signed()) as u64,
        },
        LogicOp::Sltu => match width {
            IntWidth::Word => ((a as u32) < (b as u32)) as u64,
            IntWidth::Double => (a < b) as u64,
        },
    }
}

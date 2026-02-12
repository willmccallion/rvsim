//! ALU logical and comparison operations.
//!
//! Implements bitwise OR, AND, XOR, and set-less-than (signed and unsigned)
//! for both RV64 and RV32 variants.
//!
//! For 32-bit comparisons (`Slt`/`Sltu` with `is32`), only the lower 32 bits
//! of each operand are considered. The result is always 0 or 1.

use crate::core::pipeline::signals::AluOp;

/// Executes a logical or comparison operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be a logic/comparison variant).
/// * `a`    - First operand (64-bit value).
/// * `b`    - Second operand (64-bit value).
/// * `is32` - If true, perform the 32-bit comparison variant for Slt/Sltu.
///
/// # Returns
///
/// The 64-bit result. Bitwise operations always use the full 64 bits
/// regardless of `is32`. Returns `0` for non-logic opcodes.
pub fn execute(op: AluOp, a: u64, b: u64, is32: bool) -> u64 {
    match op {
        AluOp::Or => a | b,
        AluOp::And => a & b,
        AluOp::Xor => a ^ b,
        AluOp::Slt => {
            if is32 {
                ((a as i32) < (b as i32)) as u64
            } else {
                ((a as i64) < (b as i64)) as u64
            }
        }
        AluOp::Sltu => {
            if is32 {
                ((a as u32) < (b as u32)) as u64
            } else {
                (a < b) as u64
            }
        }
        _ => 0,
    }
}

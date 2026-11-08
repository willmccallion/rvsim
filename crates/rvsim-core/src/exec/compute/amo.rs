//! Atomic memory operation ALU.
//!
//! Implements the read-modify-write arithmetic for RISC-V A-extension
//! atomic memory operations (AMO). Given the current memory value and
//! the register source value, computes the new value to be written back.
//!
//! Supports both 32-bit (Word) and 64-bit (Double) widths. All 32-bit
//! results are sign-extended from bit 31 to 64 bits.

use crate::isa::op::{AmoOp, MemWidth};

/// Performs an atomic ALU operation for AMO instructions.
///
/// Computes the result of an atomic operation that combines a value
/// from memory with a value from a register. Used by AMO instructions
/// (AMOSWAP, AMOADD, AMOXOR, etc.) to compute the new value to be
/// written back to memory.
///
/// # Arguments
///
/// * `op`      - The atomic operation type
/// * `mem_val` - The current value read from memory
/// * `reg_val` - The value from the source register
/// * `width`   - The width of the operation (Word or Double)
///
/// # Returns
///
/// The computed result that will be written back to memory.
/// For 32-bit operations, the result is sign-extended to 64 bits.
pub fn atomic_alu(op: AmoOp, mem_val: u64, reg_val: u64, width: MemWidth) -> u64 {
    if matches!(width, MemWidth::Word) {
        let a = mem_val as i32;
        let b = reg_val as i32;
        let res = match op {
            AmoOp::Swap => b,
            AmoOp::Add => a.wrapping_add(b),
            AmoOp::Xor => a ^ b,
            AmoOp::And => a & b,
            AmoOp::Or => a | b,
            AmoOp::Min => a.min(b),
            AmoOp::Max => a.max(b),
            AmoOp::MinU => (mem_val as u32).min(reg_val as u32) as i32,
            AmoOp::MaxU => (mem_val as u32).max(reg_val as u32) as i32,
        };
        res as i64 as u64
    } else {
        let a = mem_val as i64;
        let b = reg_val as i64;
        let res = match op {
            AmoOp::Swap => b,
            AmoOp::Add => a.wrapping_add(b),
            AmoOp::Xor => a ^ b,
            AmoOp::And => a & b,
            AmoOp::Or => a | b,
            AmoOp::Min => a.min(b),
            AmoOp::Max => a.max(b),
            AmoOp::MinU => (mem_val).min(reg_val) as i64,
            AmoOp::MaxU => (mem_val).max(reg_val) as i64,
        };
        res as u64
    }
}

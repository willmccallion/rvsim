//! ALU shift operations.
//!
//! Implements shift-left logical (SLL), shift-right logical (SRL), and
//! shift-right arithmetic (SRA) for both RV64 and RV32 variants.
//!
//! Shift amounts are masked to 6 bits (0–63) for RV64 and 5 bits (0–31)
//! for RV32, per RISC-V spec §2.4. All 32-bit results are sign-extended
//! from bit 31 to 64 bits.

use crate::core::pipeline::signals::AluOp;

/// Bit mask for shift amount in RV64 (6 bits: 0-63).
const SHAMT_MASK_RV64: u64 = 0x3f;

/// Bit mask for shift amount in RV32 (5 bits: 0-31).
const SHAMT_MASK_RV32: u32 = 0x1f;

/// Executes a shift operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be a shift variant).
/// * `a`    - The value to be shifted (64-bit).
/// * `b`    - The shift amount (lower bits used, upper bits ignored).
/// * `is32` - If true, perform the 32-bit (W-suffix) variant.
///
/// # Returns
///
/// The 64-bit result. For 32-bit operations the result is sign-extended
/// from bit 31. Returns `0` for non-shift opcodes.
pub fn execute(op: AluOp, a: u64, b: u64, is32: bool) -> u64 {
    let sh6 = (b & SHAMT_MASK_RV64) as u32;
    match op {
        AluOp::Sll => {
            if is32 {
                (a as i32).wrapping_shl(b as u32 & SHAMT_MASK_RV32) as i64 as u64
            } else {
                a.wrapping_shl(sh6)
            }
        }
        AluOp::Srl => {
            if is32 {
                ((a as u32).wrapping_shr(b as u32 & SHAMT_MASK_RV32)) as i32 as i64 as u64
            } else {
                a.wrapping_shr(sh6)
            }
        }
        AluOp::Sra => {
            if is32 {
                ((a as i32) >> (b as u32 & SHAMT_MASK_RV32)) as i64 as u64
            } else {
                ((a as i64) >> sh6) as u64
            }
        }
        _ => 0,
    }
}

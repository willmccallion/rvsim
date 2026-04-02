//! ALU bit-manipulation operations (B extension: Zba, Zbb, Zbc, Zbs).
//!
//! Implements the RISC-V B-extension arithmetic and logical operations
//! for both RV64 and RV32 variants.
//!
//! Reference: RISC-V Bitmanip Extension v1.0.0.

use crate::core::pipeline::signals::AluOp;

/// Bit mask for rotate amount in RV64 (6 bits: 0-63).
const SHAMT_MASK_RV64: u32 = 0x3f;

/// Bit mask for rotate amount in RV32 (5 bits: 0-31).
const SHAMT_MASK_RV32: u32 = 0x1f;

/// Sign-extends a 32-bit result to 64 bits (standard RV64 W-suffix behavior).
const fn sext32(val: u32) -> u64 {
    val as i32 as i64 as u64
}

/// Executes a B-extension ALU operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be a B-extension variant).
/// * `a`    - First operand (rs1, 64-bit value).
/// * `b`    - Second operand (rs2 or immediate, 64-bit value).
/// * `is32` - If true, perform the 32-bit (W-suffix) variant.
///
/// # Returns
///
/// The 64-bit result. For 32-bit operations the result is sign-extended
/// from bit 31. Returns `0` for non-B-extension opcodes.
pub const fn execute(op: AluOp, a: u64, b: u64, is32: bool) -> u64 {
    match op {
        // ── Zba: Address generation ──────────────────────────────────────
        AluOp::Sh1Add => (a << 1).wrapping_add(b),
        AluOp::Sh2Add => (a << 2).wrapping_add(b),
        AluOp::Sh3Add => (a << 3).wrapping_add(b),

        AluOp::AddUw => {
            // add.uw: rd = (rs1[31:0] zero-extended to 64) + rs2
            let a_zext = a as u32 as u64;
            a_zext.wrapping_add(b)
        }

        AluOp::Sh1AddUw => {
            // sh1add.uw: rd = (rs1[31:0] zero-extended) << 1 + rs2
            let a_zext = a as u32 as u64;
            (a_zext << 1).wrapping_add(b)
        }
        AluOp::Sh2AddUw => {
            let a_zext = a as u32 as u64;
            (a_zext << 2).wrapping_add(b)
        }
        AluOp::Sh3AddUw => {
            let a_zext = a as u32 as u64;
            (a_zext << 3).wrapping_add(b)
        }

        AluOp::SlliUw => {
            // slli.uw: rd = (rs1[31:0] zero-extended) << shamt
            let a_zext = a as u32 as u64;
            a_zext << (b & SHAMT_MASK_RV64 as u64)
        }

        // ── Zbb: Basic bit manipulation ──────────────────────────────────
        AluOp::Andn => a & !b,
        AluOp::Orn => a | !b,
        AluOp::Xnor => !(a ^ b),

        AluOp::Clz => {
            if is32 {
                sext32((a as u32).leading_zeros())
            } else {
                a.leading_zeros() as u64
            }
        }
        AluOp::Ctz => {
            if is32 {
                sext32((a as u32).trailing_zeros())
            } else {
                a.trailing_zeros() as u64
            }
        }
        AluOp::Cpop => {
            if is32 {
                sext32((a as u32).count_ones())
            } else {
                a.count_ones() as u64
            }
        }

        AluOp::Max => {
            if is32 {
                let sa = a as i32;
                let sb = b as i32;
                sext32(if sa > sb { sa } else { sb } as u32)
            } else {
                let sa = a as i64;
                let sb = b as i64;
                (if sa > sb { sa } else { sb }) as u64
            }
        }
        AluOp::Maxu => {
            if is32 {
                let ua = a as u32;
                let ub = b as u32;
                sext32(if ua > ub { ua } else { ub })
            } else {
                if a > b { a } else { b }
            }
        }
        AluOp::Min => {
            if is32 {
                let sa = a as i32;
                let sb = b as i32;
                sext32(if sa < sb { sa } else { sb } as u32)
            } else {
                let sa = a as i64;
                let sb = b as i64;
                (if sa < sb { sa } else { sb }) as u64
            }
        }
        AluOp::Minu => {
            if is32 {
                let ua = a as u32;
                let ub = b as u32;
                sext32(if ua < ub { ua } else { ub })
            } else {
                if a < b { a } else { b }
            }
        }

        AluOp::SextB => {
            // Sign-extend bit 7 to XLEN
            a as i8 as i64 as u64
        }
        AluOp::SextH => {
            // Sign-extend bit 15 to XLEN
            a as i16 as i64 as u64
        }
        AluOp::Rol => {
            if is32 {
                let val = a as u32;
                let shamt = b as u32 & SHAMT_MASK_RV32;
                sext32(val.rotate_left(shamt))
            } else {
                let shamt = b as u32 & SHAMT_MASK_RV64;
                a.rotate_left(shamt)
            }
        }
        AluOp::Ror => {
            if is32 {
                let val = a as u32;
                let shamt = b as u32 & SHAMT_MASK_RV32;
                sext32(val.rotate_right(shamt))
            } else {
                let shamt = b as u32 & SHAMT_MASK_RV64;
                a.rotate_right(shamt)
            }
        }

        AluOp::OrcB => {
            // OR-combine: for each byte, if any bit is set, set all bits.
            let mut result: u64 = 0;
            let mut i = 0;
            while i < 8 {
                let byte = (a >> (i * 8)) & 0xFF;
                if byte != 0 {
                    result |= 0xFF << (i * 8);
                }
                i += 1;
            }
            result
        }

        AluOp::Rev8 => {
            // Byte-reverse the entire XLEN-wide value.
            a.swap_bytes()
        }

        // ── Zbc: Carry-less multiplication ───────────────────────────────
        AluOp::Clmul => {
            // Carry-less multiply (low half).
            let mut result: u64 = 0;
            let mut i: u32 = 0;
            while i < 64 {
                if (b >> i) & 1 != 0 {
                    result ^= a << i;
                }
                i += 1;
            }
            result
        }
        AluOp::Clmulh => {
            // Carry-less multiply (high half).
            let mut result: u64 = 0;
            let mut i: u32 = 1;
            while i < 64 {
                if (b >> i) & 1 != 0 {
                    result ^= a >> (64 - i);
                }
                i += 1;
            }
            result
        }
        AluOp::Clmulr => {
            // Carry-less multiply (reversed).
            let mut result: u64 = 0;
            let mut i: u32 = 0;
            while i < 64 {
                if (b >> i) & 1 != 0 {
                    result ^= a >> (63 - i);
                }
                i += 1;
            }
            result
        }

        // ── Zbs: Single-bit operations ───────────────────────────────────
        AluOp::Bclr => {
            // Clear bit at position b[5:0] (or b[4:0] for RV32).
            let shamt = if is32 { b as u32 & SHAMT_MASK_RV32 } else { b as u32 & SHAMT_MASK_RV64 };
            a & !(1u64 << shamt)
        }
        AluOp::Bext => {
            // Extract bit at position b[5:0] (or b[4:0] for RV32).
            let shamt = if is32 { b as u32 & SHAMT_MASK_RV32 } else { b as u32 & SHAMT_MASK_RV64 };
            (a >> shamt) & 1
        }
        AluOp::Binv => {
            // Invert bit at position b[5:0] (or b[4:0] for RV32).
            let shamt = if is32 { b as u32 & SHAMT_MASK_RV32 } else { b as u32 & SHAMT_MASK_RV64 };
            a ^ (1u64 << shamt)
        }
        AluOp::Bset => {
            // Set bit at position b[5:0] (or b[4:0] for RV32).
            let shamt = if is32 { b as u32 & SHAMT_MASK_RV32 } else { b as u32 & SHAMT_MASK_RV64 };
            a | (1u64 << shamt)
        }

        // ── Zbkb: Bitwise operations for cryptography ────────────────────

        AluOp::Brev8 => {
            // Reverse the bits within each byte of the 64-bit value.
            let mut result: u64 = 0;
            let mut i = 0;
            while i < 8 {
                let byte = ((a >> (i * 8)) & 0xFF) as u8;
                let reversed = byte.reverse_bits();
                result |= (reversed as u64) << (i * 8);
                i += 1;
            }
            result
        }

        AluOp::Pack => {
            // Pack lower halves: rd = {rs2[31:0], rs1[31:0]}
            if is32 {
                // packw: rd = sext32({rs2[15:0], rs1[15:0]})
                let lo = a as u16 as u32;
                let hi = b as u16 as u32;
                sext32(lo | (hi << 16))
            } else {
                let lo = a as u32 as u64;
                let hi = b as u32 as u64;
                lo | (hi << 32)
            }
        }

        AluOp::Packh => {
            // Pack lowest bytes: rd = {0..., rs2[7:0], rs1[7:0]}
            let lo = a & 0xFF;
            let hi = b & 0xFF;
            lo | (hi << 8)
        }

        AluOp::Packw => {
            // Pack lower halves, 32-bit: rd = sext32({rs2[15:0], rs1[15:0]})
            let lo = a as u16 as u32;
            let hi = b as u16 as u32;
            sext32(lo | (hi << 16))
        }

        // ── Zbkx: Crossbar permutations for cryptography ─────────────────

        AluOp::Xperm4 => {
            // 4-bit crossbar permutation.
            // For each 4-bit nibble i in rs2, output nibble i is
            // the nibble at index rs2_nibble[i] from rs1.
            let mut result: u64 = 0;
            let mut i = 0;
            while i < 16 {
                let idx = ((b >> (i * 4)) & 0xF) as u32;
                if idx < 16 {
                    let nibble = (a >> (idx * 4)) & 0xF;
                    result |= nibble << (i * 4);
                }
                i += 1;
            }
            result
        }

        AluOp::Xperm8 => {
            // 8-bit crossbar permutation.
            // For each byte i in rs2, output byte i is
            // the byte at index rs2_byte[i] from rs1.
            let mut result: u64 = 0;
            let mut i = 0;
            while i < 8 {
                let idx = ((b >> (i * 8)) & 0xFF) as u32;
                if idx < 8 {
                    let byte = (a >> (idx * 8)) & 0xFF;
                    result |= byte << (i * 8);
                }
                i += 1;
            }
            result
        }

        _ => 0,
    }
}

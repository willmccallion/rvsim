//! ALU bit-manipulation operations (B extension: Zba, Zbb, Zbc, Zbs).
//!
//! Implements the RISC-V B-extension arithmetic and logical operations
//! for both RV64 and RV32 variants.
//!
//! Reference: RISC-V Bitmanip Extension v1.0.0.

use crate::isa::bits::sext_word;
use crate::isa::op::{AluOp, IntWidth};

const SHAMT_MASK_RV64: u32 = 0x3f;

const SHAMT_MASK_RV32: u32 = 0x1f;

/// Executes a B-extension ALU operation.
///
/// # Arguments
///
/// * `op`   - The ALU operation to perform (must be a B-extension variant).
/// * `a`    - First operand (rs1, 64-bit value).
/// * `b`    - Second operand (rs2 or immediate, 64-bit value).
/// * `width` - `Word` for the 32-bit (W-suffix) variant.
///
/// # Returns
///
/// The 64-bit result. For 32-bit operations the result is sign-extended
/// from bit 31. Returns `0` for non-B-extension opcodes.
pub const fn execute(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        AluOp::Sh1Add => (a << 1).wrapping_add(b),
        AluOp::Sh2Add => (a << 2).wrapping_add(b),
        AluOp::Sh3Add => (a << 3).wrapping_add(b),

        AluOp::AddUw => {
            let a_zext = a as u32 as u64;
            a_zext.wrapping_add(b)
        }

        AluOp::Sh1AddUw => {
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
            let a_zext = a as u32 as u64;
            a_zext << (b & SHAMT_MASK_RV64 as u64)
        }

        AluOp::Andn => a & !b,
        AluOp::Orn => a | !b,
        AluOp::Xnor => !(a ^ b),

        AluOp::Clz => match width {
            IntWidth::Word => sext_word((a as u32).leading_zeros()),
            IntWidth::Double => a.leading_zeros() as u64,
        },
        AluOp::Ctz => match width {
            IntWidth::Word => sext_word((a as u32).trailing_zeros()),
            IntWidth::Double => a.trailing_zeros() as u64,
        },
        AluOp::Cpop => match width {
            IntWidth::Word => sext_word((a as u32).count_ones()),
            IntWidth::Double => a.count_ones() as u64,
        },

        AluOp::Max => match width {
            IntWidth::Word => {
                let sa = a as i32;
                let sb = b as i32;
                sext_word(if sa > sb { sa } else { sb } as u32)
            }
            IntWidth::Double => {
                let sa = a as i64;
                let sb = b as i64;
                (if sa > sb { sa } else { sb }) as u64
            }
        },
        AluOp::Maxu => match width {
            IntWidth::Word => {
                let ua = a as u32;
                let ub = b as u32;
                sext_word(if ua > ub { ua } else { ub })
            }
            IntWidth::Double => {
                if a > b {
                    a
                } else {
                    b
                }
            }
        },
        AluOp::Min => match width {
            IntWidth::Word => {
                let sa = a as i32;
                let sb = b as i32;
                sext_word(if sa < sb { sa } else { sb } as u32)
            }
            IntWidth::Double => {
                let sa = a as i64;
                let sb = b as i64;
                (if sa < sb { sa } else { sb }) as u64
            }
        },
        AluOp::Minu => match width {
            IntWidth::Word => {
                let ua = a as u32;
                let ub = b as u32;
                sext_word(if ua < ub { ua } else { ub })
            }
            IntWidth::Double => {
                if a < b {
                    a
                } else {
                    b
                }
            }
        },

        AluOp::SextB => a as i8 as i64 as u64,
        AluOp::SextH => a as i16 as i64 as u64,
        AluOp::Rol => match width {
            IntWidth::Word => {
                let val = a as u32;
                let shamt = b as u32 & SHAMT_MASK_RV32;
                sext_word(val.rotate_left(shamt))
            }
            IntWidth::Double => {
                let shamt = b as u32 & SHAMT_MASK_RV64;
                a.rotate_left(shamt)
            }
        },
        AluOp::Ror => match width {
            IntWidth::Word => {
                let val = a as u32;
                let shamt = b as u32 & SHAMT_MASK_RV32;
                sext_word(val.rotate_right(shamt))
            }
            IntWidth::Double => {
                let shamt = b as u32 & SHAMT_MASK_RV64;
                a.rotate_right(shamt)
            }
        },

        AluOp::OrcB => {
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

        AluOp::Rev8 => a.swap_bytes(),

        AluOp::Clmul => {
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

        AluOp::Bclr => {
            let shamt = match width {
                IntWidth::Word => b as u32 & SHAMT_MASK_RV32,
                IntWidth::Double => b as u32 & SHAMT_MASK_RV64,
            };
            a & !(1u64 << shamt)
        }
        AluOp::Bext => {
            let shamt = match width {
                IntWidth::Word => b as u32 & SHAMT_MASK_RV32,
                IntWidth::Double => b as u32 & SHAMT_MASK_RV64,
            };
            (a >> shamt) & 1
        }
        AluOp::Binv => {
            let shamt = match width {
                IntWidth::Word => b as u32 & SHAMT_MASK_RV32,
                IntWidth::Double => b as u32 & SHAMT_MASK_RV64,
            };
            a ^ (1u64 << shamt)
        }
        AluOp::Bset => {
            let shamt = match width {
                IntWidth::Word => b as u32 & SHAMT_MASK_RV32,
                IntWidth::Double => b as u32 & SHAMT_MASK_RV64,
            };
            a | (1u64 << shamt)
        }

        AluOp::Brev8 => {
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

        AluOp::Pack => match width {
            IntWidth::Word => {
                let lo = a as u16 as u32;
                let hi = b as u16 as u32;
                sext_word(lo | (hi << 16))
            }
            IntWidth::Double => {
                let lo = a as u32 as u64;
                let hi = b as u32 as u64;
                lo | (hi << 32)
            }
        },

        AluOp::Packh => {
            let lo = a & 0xFF;
            let hi = b & 0xFF;
            lo | (hi << 8)
        }

        AluOp::Packw => {
            let lo = a as u16 as u32;
            let hi = b as u16 as u32;
            sext_word(lo | (hi << 16))
        }

        AluOp::Xperm4 => {
            let mut result: u64 = 0;
            let mut i = 0;
            while i < 16 {
                let idx = ((b >> (i * 4)) & 0xF) as u32;
                let nibble = (a >> (idx * 4)) & 0xF;
                result |= nibble << (i * 4);
                i += 1;
            }
            result
        }

        AluOp::Xperm8 => {
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

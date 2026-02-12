//! Compressed Instruction Expansion.
//!
//! Provides the `expand` function which converts a 16-bit compressed instruction
//! into its 32-bit uncompressed equivalent.

use super::constants::{QUADRANT_0, QUADRANT_1, QUADRANT_2, q0, q1, q2};
use crate::isa::privileged::opcodes as sys_ops;
use crate::isa::rv64f::opcodes as fp_opcodes;
use crate::isa::rv64i::{funct3, funct7, opcodes};

/// Expands a 16-bit RVC instruction into its 32-bit equivalent.
pub fn expand(inst: u16) -> u32 {
    let op = inst & 0x3;
    let funct3 = (inst >> 13) & 0x7;

    match op {
        QUADRANT_0 => match funct3 {
            q0::C_ADDI4SPN => {
                let imm = ((inst >> 6) & 1) << 2
                    | ((inst >> 5) & 1) << 3
                    | ((inst >> 11) & 0x3) << 4
                    | ((inst >> 7) & 0xF) << 6;
                if imm == 0 {
                    return 0;
                }
                let rd = 8 + ((inst >> 2) & 0x7) as u32;
                (imm as u32) << 20
                    | (2 << 15)
                    | (funct3::ADD_SUB << 12)
                    | (rd << 7)
                    | opcodes::OP_IMM
            }
            q0::C_FLD => {
                let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 0x3) << 6;
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let rd = 8 + ((inst >> 2) & 0x7) as u32;
                (imm as u32) << 20
                    | (rs1 << 15)
                    | (funct3::LD << 12)
                    | (rd << 7)
                    | fp_opcodes::OP_LOAD_FP
            }
            q0::C_LW => {
                let imm =
                    ((inst >> 6) & 1) << 2 | ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 1) << 6;
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let rd = 8 + ((inst >> 2) & 0x7) as u32;
                (imm as u32) << 20 | (rs1 << 15) | (funct3::LW << 12) | (rd << 7) | opcodes::OP_LOAD
            }
            q0::C_LD => {
                let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 0x3) << 6;
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let rd = 8 + ((inst >> 2) & 0x7) as u32;
                (imm as u32) << 20 | (rs1 << 15) | (funct3::LD << 12) | (rd << 7) | opcodes::OP_LOAD
            }
            q0::C_FSD => {
                let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 0x3) << 6;
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let rs2 = 8 + ((inst >> 2) & 0x7) as u32;
                let imm_low = (imm & 0x1F) as u32;
                let imm_high = (imm >> 5) as u32;
                (imm_high << 25)
                    | (rs2 << 20)
                    | (rs1 << 15)
                    | (funct3::SD << 12)
                    | (imm_low << 7)
                    | fp_opcodes::OP_STORE_FP
            }
            q0::C_SW => {
                let imm =
                    ((inst >> 6) & 1) << 2 | ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 1) << 6;
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let rs2 = 8 + ((inst >> 2) & 0x7) as u32;
                let imm_low = (imm & 0x1F) as u32;
                let imm_high = (imm >> 5) as u32;
                (imm_high << 25)
                    | (rs2 << 20)
                    | (rs1 << 15)
                    | (funct3::SW << 12)
                    | (imm_low << 7)
                    | opcodes::OP_STORE
            }
            q0::C_SD => {
                let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 0x3) << 6;
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let rs2 = 8 + ((inst >> 2) & 0x7) as u32;
                let imm_low = (imm & 0x1F) as u32;
                let imm_high = (imm >> 5) as u32;
                (imm_high << 25)
                    | (rs2 << 20)
                    | (rs1 << 15)
                    | (funct3::SD << 12)
                    | (imm_low << 7)
                    | opcodes::OP_STORE
            }
            _ => 0,
        },

        QUADRANT_1 => match funct3 {
            q1::C_ADDI => {
                let imm = sign_extend((((inst >> 2) & 0x1F) | ((inst >> 12) & 1) << 5) as u32, 6);
                let rd = ((inst >> 7) & 0x1F) as u32;
                ((imm & 0xFFF) << 20)
                    | (rd << 15)
                    | (funct3::ADD_SUB << 12)
                    | (rd << 7)
                    | opcodes::OP_IMM
            }
            q1::C_ADDIW => {
                let imm = sign_extend((((inst >> 2) & 0x1F) | ((inst >> 12) & 1) << 5) as u32, 6);
                let rd = ((inst >> 7) & 0x1F) as u32;
                if rd == 0 {
                    return 0;
                }
                ((imm & 0xFFF) << 20)
                    | (rd << 15)
                    | (funct3::ADD_SUB << 12)
                    | (rd << 7)
                    | opcodes::OP_IMM_32
            }
            q1::C_LI => {
                let imm = sign_extend((((inst >> 2) & 0x1F) | ((inst >> 12) & 1) << 5) as u32, 6);
                let rd = ((inst >> 7) & 0x1F) as u32;
                ((imm & 0xFFF) << 20)
                    | (0 << 15)
                    | (funct3::ADD_SUB << 12)
                    | (rd << 7)
                    | opcodes::OP_IMM
            }
            q1::C_LUI_ADDI16SP => {
                let rd = ((inst >> 7) & 0x1F) as u32;
                if rd == 2 {
                    let imm = sign_extend(
                        (((inst >> 6) & 1) << 4
                            | ((inst >> 2) & 1) << 5
                            | ((inst >> 5) & 1) << 6
                            | ((inst >> 3) & 3) << 7
                            | ((inst >> 12) & 1) << 9) as u32,
                        10,
                    );
                    if imm == 0 {
                        return 0;
                    }
                    ((imm & 0xFFF) << 20)
                        | (2 << 15)
                        | (funct3::ADD_SUB << 12)
                        | (2 << 7)
                        | opcodes::OP_IMM
                } else {
                    let imm =
                        sign_extend((((inst >> 2) & 0x1F) | ((inst >> 12) & 1) << 5) as u32, 6);
                    if imm == 0 {
                        return 0;
                    }
                    (imm << 12) | (rd << 7) | opcodes::OP_LUI
                }
            }
            q1::C_MISC_ALU => {
                let bit12 = (inst >> 12) & 1;
                let funct2 = (inst >> 10) & 0x3;
                let rd = 8 + ((inst >> 7) & 0x7) as u32;
                let imm = sign_extend((((inst >> 2) & 0x1F) | ((inst >> 12) & 1) << 5) as u32, 6);

                match funct2 {
                    0 => {
                        ((imm & 0x3F) << 20)
                            | (rd << 15)
                            | (funct3::SRL_SRA << 12)
                            | (rd << 7)
                            | opcodes::OP_IMM
                    }
                    1 => {
                        (funct7::SRA << 25)
                            | ((imm & 0x3F) << 20)
                            | (rd << 15)
                            | (funct3::SRL_SRA << 12)
                            | (rd << 7)
                            | opcodes::OP_IMM
                    }
                    2 => {
                        ((imm & 0xFFF) << 20)
                            | (rd << 15)
                            | (funct3::AND << 12)
                            | (rd << 7)
                            | opcodes::OP_IMM
                    }
                    3 => {
                        let sub_op = (inst >> 5) & 0x3;
                        let rs2 = 8 + ((inst >> 2) & 0x7) as u32;
                        match (bit12, sub_op) {
                            (0, 0) => {
                                (funct7::SUB << 25)
                                    | (rs2 << 20)
                                    | (rd << 15)
                                    | (funct3::ADD_SUB << 12)
                                    | (rd << 7)
                                    | opcodes::OP_REG
                            }
                            (0, 1) => {
                                (rs2 << 20)
                                    | (rd << 15)
                                    | (funct3::XOR << 12)
                                    | (rd << 7)
                                    | opcodes::OP_REG
                            }
                            (0, 2) => {
                                (rs2 << 20)
                                    | (rd << 15)
                                    | (funct3::OR << 12)
                                    | (rd << 7)
                                    | opcodes::OP_REG
                            }
                            (0, 3) => {
                                (rs2 << 20)
                                    | (rd << 15)
                                    | (funct3::AND << 12)
                                    | (rd << 7)
                                    | opcodes::OP_REG
                            }
                            (1, 0) => {
                                (funct7::SUB << 25)
                                    | (rs2 << 20)
                                    | (rd << 15)
                                    | (funct3::ADD_SUB << 12)
                                    | (rd << 7)
                                    | opcodes::OP_REG_32
                            }
                            (1, 1) => {
                                (rs2 << 20)
                                    | (rd << 15)
                                    | (funct3::ADD_SUB << 12)
                                    | (rd << 7)
                                    | opcodes::OP_REG_32
                            }
                            _ => 0,
                        }
                    }
                    _ => 0,
                }
            }
            q1::C_J => {
                let offset = sign_extend(
                    (((inst >> 3) & 0x7) << 1
                        | ((inst >> 11) & 1) << 4
                        | ((inst >> 2) & 1) << 5
                        | ((inst >> 7) & 1) << 6
                        | ((inst >> 6) & 1) << 7
                        | ((inst >> 9) & 3) << 8
                        | ((inst >> 8) & 1) << 10
                        | ((inst >> 12) & 1) << 11) as u32,
                    12,
                );
                let imm20 = (offset >> 20) & 1;
                let imm10_1 = (offset >> 1) & 0x3FF;
                let imm11 = (offset >> 11) & 1;
                let imm19_12 = (offset >> 12) & 0xFF;
                (imm20 << 31)
                    | (imm19_12 << 12)
                    | (imm11 << 20)
                    | (imm10_1 << 21)
                    | (0 << 7)
                    | opcodes::OP_JAL
            }
            q1::C_BEQZ => {
                let offset = sign_extend(
                    (((inst >> 3) & 0x3) << 1
                        | ((inst >> 10) & 0x3) << 3
                        | ((inst >> 2) & 1) << 5
                        | ((inst >> 5) & 0x3) << 6
                        | ((inst >> 12) & 1) << 8) as u32,
                    9,
                );
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let imm12 = (offset >> 12) & 1;
                let imm10_5 = (offset >> 5) & 0x3F;
                let imm4_1 = (offset >> 1) & 0xF;
                let imm11 = (offset >> 11) & 1;
                (imm12 << 31)
                    | (imm10_5 << 25)
                    | (0 << 20)
                    | (rs1 << 15)
                    | (funct3::BEQ << 12)
                    | (imm4_1 << 8)
                    | (imm11 << 7)
                    | opcodes::OP_BRANCH
            }
            q1::C_BNEZ => {
                let offset = sign_extend(
                    (((inst >> 3) & 0x3) << 1
                        | ((inst >> 10) & 0x3) << 3
                        | ((inst >> 2) & 1) << 5
                        | ((inst >> 5) & 0x3) << 6
                        | ((inst >> 12) & 1) << 8) as u32,
                    9,
                );
                let rs1 = 8 + ((inst >> 7) & 0x7) as u32;
                let imm12 = (offset >> 12) & 1;
                let imm10_5 = (offset >> 5) & 0x3F;
                let imm4_1 = (offset >> 1) & 0xF;
                let imm11 = (offset >> 11) & 1;
                (imm12 << 31)
                    | (imm10_5 << 25)
                    | (0 << 20)
                    | (rs1 << 15)
                    | (funct3::BNE << 12)
                    | (imm4_1 << 8)
                    | (imm11 << 7)
                    | opcodes::OP_BRANCH
            }
            _ => 0,
        },

        QUADRANT_2 => match funct3 {
            q2::C_SLLI => {
                let imm = ((inst >> 2) & 0x1F) | ((inst >> 12) & 1) << 5;
                let rd = ((inst >> 7) & 0x1F) as u32;
                if rd == 0 {
                    return 0;
                }
                (imm as u32) << 20 | (rd << 15) | (funct3::SLL << 12) | (rd << 7) | opcodes::OP_IMM
            }
            q2::C_FLDSP => {
                let imm =
                    ((inst >> 12) & 1) << 5 | ((inst >> 5) & 0x3) << 3 | ((inst >> 2) & 0x7) << 6;
                let rd = ((inst >> 7) & 0x1F) as u32;
                (imm as u32) << 20
                    | (2 << 15)
                    | (funct3::LD << 12)
                    | (rd << 7)
                    | fp_opcodes::OP_LOAD_FP
            }
            q2::C_LWSP => {
                let imm =
                    ((inst >> 12) & 1) << 5 | ((inst >> 4) & 0x7) << 2 | ((inst >> 2) & 0x3) << 6;
                let rd = ((inst >> 7) & 0x1F) as u32;
                if rd == 0 {
                    return 0;
                }
                (imm as u32) << 20 | (2 << 15) | (funct3::LW << 12) | (rd << 7) | opcodes::OP_LOAD
            }
            q2::C_LDSP => {
                let imm =
                    ((inst >> 12) & 1) << 5 | ((inst >> 5) & 0x3) << 3 | ((inst >> 2) & 0x7) << 6;
                let rd = ((inst >> 7) & 0x1F) as u32;
                if rd == 0 {
                    return 0;
                }
                (imm as u32) << 20 | (2 << 15) | (funct3::LD << 12) | (rd << 7) | opcodes::OP_LOAD
            }
            q2::C_MISC_ALU => {
                let bit12 = (inst >> 12) & 1;
                let rs2 = ((inst >> 2) & 0x1F) as u32;
                let rs1 = ((inst >> 7) & 0x1F) as u32;
                if bit12 == 0 {
                    if rs2 == 0 {
                        if rs1 == 0 {
                            return 0;
                        }
                        (rs1 << 15) | (funct3::ADD_SUB << 12) | opcodes::OP_JALR
                    } else {
                        (rs2 << 20)
                            | (0 << 15)
                            | (funct3::ADD_SUB << 12)
                            | (rs1 << 7)
                            | opcodes::OP_REG
                    }
                } else {
                    if rs2 == 0 {
                        if rs1 == 0 {
                            sys_ops::EBREAK
                        } else {
                            (rs1 << 15) | (funct3::ADD_SUB << 12) | (1 << 7) | opcodes::OP_JALR
                        }
                    } else {
                        (rs2 << 20)
                            | (rs1 << 15)
                            | (funct3::ADD_SUB << 12)
                            | (rs1 << 7)
                            | opcodes::OP_REG
                    }
                }
            }
            q2::C_FSDSP => {
                let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 7) & 0x7) << 6;
                let rs2 = ((inst >> 2) & 0x1F) as u32;
                let imm_low = (imm & 0x1F) as u32;
                let imm_high = (imm >> 5) as u32;
                (imm_high << 25)
                    | (rs2 << 20)
                    | (2 << 15)
                    | (funct3::SD << 12)
                    | (imm_low << 7)
                    | fp_opcodes::OP_STORE_FP
            }
            q2::C_SWSP => {
                let imm = ((inst >> 9) & 0xF) << 2 | ((inst >> 7) & 0x3) << 6;
                let rs2 = ((inst >> 2) & 0x1F) as u32;
                let imm_low = (imm & 0x1F) as u32;
                let imm_high = (imm >> 5) as u32;
                (imm_high << 25)
                    | (rs2 << 20)
                    | (2 << 15)
                    | (funct3::SW << 12)
                    | (imm_low << 7)
                    | opcodes::OP_STORE
            }
            q2::C_SDSP => {
                let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 7) & 0x7) << 6;
                let rs2 = ((inst >> 2) & 0x1F) as u32;
                let imm_low = (imm & 0x1F) as u32;
                let imm_high = (imm >> 5) as u32;
                (imm_high << 25)
                    | (rs2 << 20)
                    | (2 << 15)
                    | (funct3::SD << 12)
                    | (imm_low << 7)
                    | opcodes::OP_STORE
            }
            _ => 0,
        },
        _ => 0,
    }
}

/// Sign-extends a value from `bits` width to 32 bits.
///
/// Performs arithmetic right shift to propagate the sign bit through
/// the upper bits of the result. Used for expanding compressed instruction
/// immediates to their 32-bit equivalents.
///
/// # Arguments
///
/// * `val` - The value to sign-extend (only lower `bits` are valid).
/// * `bits` - The number of valid bits in `val` (must be <= 32).
///
/// # Returns
///
/// The sign-extended 32-bit value.
fn sign_extend(val: u32, bits: u32) -> u32 {
    let shift = 32 - bits;
    ((val << shift) as i32 >> shift) as u32
}

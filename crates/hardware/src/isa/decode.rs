//! RISC-V Instruction Decoder.
//!
//! This module handles the decoding of 32-bit RISC-V instruction encodings into
//! a structured `Decoded` format. It extracts opcodes, register indices, function
//! codes, and handles the sign-extension of immediate values for all instruction
//! formats (R, I, S, B, U, J).

use crate::isa::instruction::{Decoded, InstructionBits};
use crate::isa::rv64f::opcodes as fp_opcodes;
use crate::isa::rv64i::opcodes;

/// Total width of a RISC-V instruction in bits.
const INSTRUCTION_WIDTH: u32 = 32;

/// Bit shift for extracting I-Type immediate field (bits 20-31).
///
/// I-Type format: `imm[11:0] | rs1 | funct3 | rd | opcode`
/// The immediate occupies the upper 12 bits and is sign-extended.
const I_IMM_SHIFT: u32 = 20;

/// Bit shift for extracting S-Type immediate low field (bits 7-11).
///
/// S-Type format: `imm[11:5] | rs2 | rs1 | funct3 | imm[4:0] | opcode`
/// The immediate is split across two non-contiguous fields.
const S_IMM_LOW_SHIFT: u32 = 7;

/// Bit mask for S-Type immediate low field (5 bits: imm[4:0]).
const S_IMM_LOW_MASK: u32 = 0x1F;

/// Bit shift for extracting S-Type immediate high field (bits 25-31).
const S_IMM_HIGH_SHIFT: u32 = 25;

/// Bit mask for S-Type immediate high field (7 bits: imm[11:5]).
const S_IMM_HIGH_MASK: u32 = 0x7F;

/// Bit shift for combining S-Type immediate fields after extraction.
const S_IMM_COMBINED_SHIFT: u32 = 5;

/// Total number of bits in S-Type immediate (12 bits).
const S_IMM_BITS: u32 = 12;

/// Bit shift for extracting B-Type immediate bit 11 (bit 7 of instruction).
///
/// B-Type format: `imm[12] | imm[10:5] | rs2 | rs1 | funct3 | imm[4:1] | imm[11] | opcode`
/// The immediate represents a signed offset in multiples of 2 (even addresses only).
const B_IMM_11_SHIFT: u32 = 7;

/// Bit mask for B-Type immediate bit 11.
const B_IMM_11_MASK: u32 = 1;

/// Bit shift for extracting B-Type immediate bits 4-1 (bits 8-11 of instruction).
const B_IMM_4_1_SHIFT: u32 = 8;

/// Bit mask for B-Type immediate bits 4-1 (4 bits).
const B_IMM_4_1_MASK: u32 = 0xF;

/// Bit shift for extracting B-Type immediate bits 10-5 (bits 25-30 of instruction).
const B_IMM_10_5_SHIFT: u32 = 25;

/// Bit mask for B-Type immediate bits 10-5 (6 bits).
const B_IMM_10_5_MASK: u32 = 0x3F;

/// Bit shift for extracting B-Type immediate bit 12 (bit 31 of instruction).
const B_IMM_12_SHIFT: u32 = 31;

/// Bit mask for B-Type immediate bit 12 (sign bit).
const B_IMM_12_MASK: u32 = 1;

/// Total number of bits in B-Type immediate (13 bits, sign-extended).
const B_IMM_BITS: u32 = 13;

/// Bit position of bit 12 in the reconstructed B-Type immediate.
const B_IMM_12_POS: u32 = 12;

/// Bit position of bit 11 in the reconstructed B-Type immediate.
const B_IMM_11_POS: u32 = 11;

/// Bit position of bits 10-5 in the reconstructed B-Type immediate.
const B_IMM_10_5_POS: u32 = 5;

/// Bit position of bits 4-1 in the reconstructed B-Type immediate.
const B_IMM_4_1_POS: u32 = 1;

/// Bit mask for extracting U-Type immediate field (bits 12-31).
///
/// U-Type format: `imm[31:12] | rd | opcode`
/// The immediate is left-shifted by 12 bits in the final value (no sign extension).
const U_IMM_MASK: u32 = 0xFFFFF000;

/// Bit shift for extracting J-Type immediate bits 19-12 (bits 12-19 of instruction).
///
/// J-Type format: `imm[20] | imm[10:1] | imm[11] | imm[19:12] | rd | opcode`
/// The immediate represents a signed offset in multiples of 2 (even addresses only).
const J_IMM_19_12_SHIFT: u32 = 12;

/// Bit mask for J-Type immediate bits 19-12 (8 bits).
const J_IMM_19_12_MASK: u32 = 0xFF;

/// Bit shift for extracting J-Type immediate bit 11 (bit 20 of instruction).
const J_IMM_11_SHIFT: u32 = 20;

/// Bit mask for J-Type immediate bit 11.
const J_IMM_11_MASK: u32 = 1;

/// Bit shift for extracting J-Type immediate bits 10-1 (bits 21-30 of instruction).
const J_IMM_10_1_SHIFT: u32 = 21;

/// Bit mask for J-Type immediate bits 10-1 (10 bits).
const J_IMM_10_1_MASK: u32 = 0x3FF;

/// Bit shift for extracting J-Type immediate bit 20 (bit 31 of instruction).
const J_IMM_20_SHIFT: u32 = 31;

/// Bit mask for J-Type immediate bit 20 (sign bit).
const J_IMM_20_MASK: u32 = 1;

/// Total number of bits in J-Type immediate (21 bits, sign-extended).
const J_IMM_BITS: u32 = 21;

/// Bit position of bit 20 in the reconstructed J-Type immediate.
const J_IMM_20_POS: u32 = 20;

/// Bit position of bits 19-12 in the reconstructed J-Type immediate.
const J_IMM_19_12_POS: u32 = 12;

/// Bit position of bit 11 in the reconstructed J-Type immediate.
const J_IMM_11_POS: u32 = 11;

/// Bit position of bits 10-1 in the reconstructed J-Type immediate.
const J_IMM_10_1_POS: u32 = 1;

/// Decodes a RISC-V instruction into its component fields.
///
/// Extracts opcode, register fields, function codes, and sign-extended
/// immediate values from a 32-bit instruction encoding.
///
/// # Arguments
///
/// * `inst` - The 32-bit instruction encoding to decode
///
/// # Returns
///
/// A `Decoded` structure containing all extracted instruction fields.
pub fn decode(inst: u32) -> Decoded {
    let opcode = inst.opcode();

    let imm = match opcode {
        opcodes::OP_IMM
        | opcodes::OP_LOAD
        | opcodes::OP_JALR
        | opcodes::OP_IMM_32
        | fp_opcodes::OP_LOAD_FP => decode_i_type_imm(inst),

        opcodes::OP_STORE | fp_opcodes::OP_STORE_FP => decode_s_type_imm(inst),
        opcodes::OP_BRANCH => decode_b_type_imm(inst),
        opcodes::OP_LUI | opcodes::OP_AUIPC => decode_u_type_imm(inst),
        opcodes::OP_JAL => decode_j_type_imm(inst),

        _ => 0,
    };

    Decoded {
        raw: inst,
        opcode,
        rd: InstructionBits::rd(&inst),
        rs1: InstructionBits::rs1(&inst),
        rs2: InstructionBits::rs2(&inst),
        funct3: InstructionBits::funct3(&inst),
        funct7: InstructionBits::funct7(&inst),
        imm,
    }
}

/// Decodes the immediate value for I-Type instructions.
///
/// I-Type format: `imm[11:0] | rs1 | funct3 | rd | opcode`
/// Used for Load, JALR, and Immediate Arithmetic instructions.
fn decode_i_type_imm(inst: u32) -> i64 {
    ((inst as i32) >> I_IMM_SHIFT) as i64
}

/// Decodes the immediate value for S-Type instructions.
///
/// S-Type format: `imm[11:5] | rs2 | rs1 | funct3 | imm[4:0] | opcode`
/// Used for Store instructions.
fn decode_s_type_imm(inst: u32) -> i64 {
    let low = (inst >> S_IMM_LOW_SHIFT) & S_IMM_LOW_MASK;
    let high = (inst >> S_IMM_HIGH_SHIFT) & S_IMM_HIGH_MASK;
    let combined = (high << S_IMM_COMBINED_SHIFT) | low;
    sign_extend(combined, S_IMM_BITS)
}

/// Decodes the immediate value for B-Type instructions.
///
/// B-Type format: `imm[12] | imm[10:5] | rs2 | rs1 | funct3 | imm[4:1] | imm[11] | opcode`
/// Used for Conditional Branch instructions. The immediate represents an even offset.
fn decode_b_type_imm(inst: u32) -> i64 {
    let bit_11 = (inst >> B_IMM_11_SHIFT) & B_IMM_11_MASK;
    let bits_4_1 = (inst >> B_IMM_4_1_SHIFT) & B_IMM_4_1_MASK;
    let bits_10_5 = (inst >> B_IMM_10_5_SHIFT) & B_IMM_10_5_MASK;
    let bit_12 = (inst >> B_IMM_12_SHIFT) & B_IMM_12_MASK;

    let combined = (bit_12 << B_IMM_12_POS)
        | (bit_11 << B_IMM_11_POS)
        | (bits_10_5 << B_IMM_10_5_POS)
        | (bits_4_1 << B_IMM_4_1_POS);
    sign_extend(combined, B_IMM_BITS)
}

/// Decodes the immediate value for U-Type instructions.
///
/// U-Type format: `imm[31:12] | rd | opcode`
/// Used for LUI and AUIPC.
fn decode_u_type_imm(inst: u32) -> i64 {
    ((inst & U_IMM_MASK) as i32) as i64
}

/// Decodes the immediate value for J-Type instructions.
///
/// J-Type format: `imm[20] | imm[10:1] | imm[11] | imm[19:12] | rd | opcode`
/// Used for JAL (Unconditional Jump).
fn decode_j_type_imm(inst: u32) -> i64 {
    let bits_19_12 = (inst >> J_IMM_19_12_SHIFT) & J_IMM_19_12_MASK;
    let bit_11 = (inst >> J_IMM_11_SHIFT) & J_IMM_11_MASK;
    let bits_10_1 = (inst >> J_IMM_10_1_SHIFT) & J_IMM_10_1_MASK;
    let bit_20 = (inst >> J_IMM_20_SHIFT) & J_IMM_20_MASK;

    let combined = (bit_20 << J_IMM_20_POS)
        | (bits_19_12 << J_IMM_19_12_POS)
        | (bit_11 << J_IMM_11_POS)
        | (bits_10_1 << J_IMM_10_1_POS);
    sign_extend(combined, J_IMM_BITS)
}

/// Sign extends a value of `bits` width to a 64-bit signed integer.
///
/// # Arguments
///
/// * `val` - The value to extend.
/// * `bits` - The number of valid bits in `val`.
fn sign_extend(val: u32, bits: u32) -> i64 {
    let shift = INSTRUCTION_WIDTH - bits;
    ((val as i32) << shift >> shift) as i64
}

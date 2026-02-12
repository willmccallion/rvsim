//! RISC-V Base Integer (I) Opcodes.
//!
//! Defines the major opcodes (bits 6-0) for the base integer instruction set.

/// Load instructions (LB, LH, LW, LD, etc.).
pub const OP_LOAD: u32 = 0b0000011;

/// Immediate arithmetic instructions (ADDI, ANDI, SLLI, etc.).
pub const OP_IMM: u32 = 0b0010011;

/// Add Upper Immediate to PC (AUIPC).
pub const OP_AUIPC: u32 = 0b0010111;

/// 32-bit Immediate arithmetic (ADDIW, SLLIW, etc.) - RV64 only.
pub const OP_IMM_32: u32 = 0b0011011;

/// Store instructions (SB, SH, SW, SD).
pub const OP_STORE: u32 = 0b0100011;

/// Register-Register arithmetic (ADD, SUB, SLL, etc.).
pub const OP_REG: u32 = 0b0110011;

/// Load Upper Immediate (LUI).
pub const OP_LUI: u32 = 0b0110111;

/// 32-bit Register-Register arithmetic (ADDW, SUBW, etc.) - RV64 only.
pub const OP_REG_32: u32 = 0b0111011;

/// Conditional Branch instructions (BEQ, BNE, etc.).
pub const OP_BRANCH: u32 = 0b1100011;

/// Jump and Link Register (JALR).
pub const OP_JALR: u32 = 0b1100111;

/// Jump and Link (JAL).
pub const OP_JAL: u32 = 0b1101111;

/// Memory ordering instructions (FENCE, FENCE.I).
pub const OP_MISC_MEM: u32 = 0b0001111;

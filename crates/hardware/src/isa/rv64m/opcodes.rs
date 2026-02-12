//! RISC-V Multiply/Divide Extension (M) Opcodes.
//!
//! The M extension shares the `OP_REG` opcode with base integer instructions.
//! It is distinguished by the `funct7` field having the value 1.

/// M-Extension selector in funct7 field.
/// When `opcode` is `OP_REG` and `funct7` is `M_EXTENSION`, the instruction
/// is a multiply or divide operation.
pub const M_EXTENSION: u32 = 0b0000001;

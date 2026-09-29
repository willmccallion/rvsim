//! RISC-V Multiply/Divide Extension (M).
//!
//! The 'M' extension adds instructions for integer multiplication and division.
//! These instructions share the `OP_REG` opcode with base integer arithmetic
//! but are distinguished by the `funct7` field being set to 1 (`M_EXTENSION`).
//!
//! # Structure
//!
//! - `opcodes`: M-extension specific constants.
//! - `funct3`: Function codes identifying specific M-ops (MUL, DIV, etc.).

/// Function code 3 definitions for multiply/divide operations.
pub mod funct3;

/// Multiply/divide extension opcodes.
pub mod opcodes;

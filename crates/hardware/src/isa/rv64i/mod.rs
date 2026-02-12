//! RISC-V Base Integer Instruction Set (I).
//!
//! Defines the fundamental integer instructions required by any RISC-V implementation.
//!
//! # Structure
//!
//! - `opcodes`: Major opcodes (Load, Store, Branch, Jal, OpImm, OpReg, etc.).
//! - `funct3`: Minor opcodes distinguishing instructions within a major opcode.
//! - `funct7`: Additional opcode bits for R-type instructions.
//! - `decode`: Logic to decode raw instruction bits into a structured format.

/// Function code 3 definitions for base integer operations.
pub mod funct3;

/// Function code 7 definitions for base integer operations.
pub mod funct7;

/// Base integer instruction set opcodes.
pub mod opcodes;

//! RISC-V Base Integer Instruction Set (I).
//!
//! Defines the fundamental integer instructions required by any RISC-V implementation.
//!
//! # Structure
//!
//! - `opcodes`: Major opcodes (Load, Store, Branch, Jal, `OpImm`, `OpReg`, etc.).
//! - `funct3`: Minor opcodes distinguishing instructions within a major opcode.
//! - `funct7`: Additional opcode bits for R-type instructions.
//! - `decode`: Logic to decode raw instruction bits into a structured format.

pub mod funct3;

pub mod funct7;

pub mod opcodes;

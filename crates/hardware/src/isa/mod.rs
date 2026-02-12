//! Instruction Set Architecture (ISA) Definitions.
//!
//! Contains definitions for opcodes, function codes, and decoding logic, organized
//! by RISC-V extension.
//!
//! # Extensions
//!
//! * `rv64i`: Base Integer Instruction Set (64-bit).
//! * `rv64m`: Standard Extension for Integer Multiplication and Division.
//! * `rv64a`: Standard Extension for Atomic Instructions.
//! * `rv64f`: Standard Extension for Single-Precision Floating-Point.
//! * `rv64d`: Standard Extension for Double-Precision Floating-Point.
//! * `rvc`: Standard Extension for Compressed Instructions.
//! * `privileged`: Privileged Architecture (CSRs, Traps).

/// Application Binary Interface (ABI) register name mappings.
pub mod abi;

/// Instruction decoding logic for all RISC-V instruction formats.
pub mod decode;

/// Instruction disassembler for debug tracing and diagnostics.
pub mod disasm;

/// Instruction encoding structures and bit extraction utilities.
pub mod instruction;

/// Privileged architecture definitions (CSRs, traps, system instructions).
pub mod privileged;

/// Atomic memory operations extension (AMO instructions).
pub mod rv64a;

/// Double-precision floating-point extension (64-bit FP operations).
pub mod rv64d;

/// Single-precision floating-point extension (32-bit FP operations).
pub mod rv64f;

/// Base integer instruction set (64-bit RISC-V core instructions).
pub mod rv64i;

/// Integer multiply/divide extension (MUL, DIV, REM instructions).
pub mod rv64m;

/// Compressed instruction extension (16-bit instruction encoding).
pub mod rvc;

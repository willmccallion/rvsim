//! # ISA Unit Tests
//!
//! This module contains unit tests for the Instruction Set Architecture (ISA) implementation.
//! It covers instruction decoding, disassembly, and the RVC (Compressed) extension.

/// Instruction decoding property tests.
///
/// This module verifies that the decoder correctly extracts fields such as
/// opcodes, register indices, and sign-extended immediates for all
/// supported RV64GC instruction formats.
pub mod decode_properties;

/// Instruction disassembler unit tests.
///
/// This module verifies that the disassembler correctly converts raw instruction
/// encodings into human-readable mnemonics for RV64I, RV64M, RV64A, RV64F/D,
/// and privileged instructions.
pub mod disasm;

/// RISC-V Compressed (RVC) instruction set extension tests.
///
/// This module contains tests for the decompression and mapping of 16-bit
/// compressed instructions to their 32-bit equivalents, covering all
/// three quadrants (Q0, Q1, Q2) of the RVC encoding space.
pub mod rvc;

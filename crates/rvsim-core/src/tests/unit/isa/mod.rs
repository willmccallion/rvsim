//! # ISA Unit Tests
//!
//! This module contains unit tests for the Instruction Set Architecture (ISA) implementation.
//! It covers instruction decoding, disassembly, and the RVC (Compressed) extension.

/// RISC-V Compressed (RVC) instruction set extension tests.
///
/// This module contains tests for the decompression and mapping of 16-bit
/// compressed instructions to their 32-bit equivalents, covering all
/// three quadrants (Q0, Q1, Q2) of the RVC encoding space.
pub mod rvc;

pub mod decode_properties;
pub mod disasm;
pub mod disasm_all_instructions;
pub mod disasm_vec;
pub mod fence;

//! RISC-V Compressed Extension (C).
//!
//! The C extension provides 16-bit compressed instructions to improve code density.
//!
//! # Structure
//!
//! - `constants`: Quadrant and opcode definitions for compressed instructions.
//! - `expand`: Logic to expand 16-bit compressed instructions into their 32-bit equivalents.

/// Compressed instruction quadrant and opcode constants.
pub mod constants;

/// Logic to expand 16-bit compressed instructions into 32-bit equivalents.
pub mod expand;

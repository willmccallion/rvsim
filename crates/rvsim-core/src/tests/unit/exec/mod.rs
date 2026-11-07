//! Instruction semantics: ALU, FPU, atomics, misaligned accesses.

pub mod alu;
pub mod amo;
pub mod branch;
pub mod decode;
pub mod fpu;
pub mod misaligned;

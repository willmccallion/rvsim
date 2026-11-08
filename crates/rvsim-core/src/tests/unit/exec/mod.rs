//! Instruction semantics: ALU, FPU, atomics, misaligned accesses.

pub mod alu;
pub mod amo;
pub mod branch;
pub mod decode;
pub mod disasm;
pub mod disasm_all_instructions;
pub mod disasm_objdump;
pub mod disasm_vec;
pub mod fpu;
pub mod misaligned;

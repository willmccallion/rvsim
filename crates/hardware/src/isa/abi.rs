//! RISC-V Application Binary Interface (ABI) register name constants.
//!
//! Defines standard RISC-V ABI register names and their corresponding
//! register indices for use in system calls and function calling conventions.

/// Register x0 (zero register, always zero).
pub const REG_ZERO: usize = 0;
/// Register x1 (return address, ra).
pub const REG_RA: usize = 1;
/// Register x2 (stack pointer, sp).
pub const REG_SP: usize = 2;
/// Register x10 (first argument/return value, a0).
pub const REG_A0: usize = 10;
/// Register x11 (second argument, a1).
pub const REG_A1: usize = 11;
/// Register x12 (third argument, a2).
pub const REG_A2: usize = 12;
/// Register x17 (system call number, a7).
pub const REG_A7: usize = 17;

//! RISC-V Application Binary Interface (ABI) register name constants.
//!
//! Defines standard RISC-V ABI register names and their corresponding
//! [`RegIdx`] values for use in system calls and function calling conventions.

use crate::common::RegIdx;

/// Register x0 (zero register, always zero).
pub const REG_ZERO: RegIdx = RegIdx::new(0);
/// Register x1 (return address, ra).
pub const REG_RA: RegIdx = RegIdx::new(1);
/// Register x2 (stack pointer, sp).
pub const REG_SP: RegIdx = RegIdx::new(2);
/// Register x5 (alternate link register, t0).
pub const REG_T0: RegIdx = RegIdx::new(5);
/// Register x10 (first argument/return value, a0).
pub const REG_A0: RegIdx = RegIdx::new(10);
/// Register x11 (second argument, a1).
pub const REG_A1: RegIdx = RegIdx::new(11);
/// Register x12 (third argument, a2).
pub const REG_A2: RegIdx = RegIdx::new(12);
/// Register x17 (system call number, a7).
pub const REG_A7: RegIdx = RegIdx::new(17);

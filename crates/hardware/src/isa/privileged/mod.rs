//! Privileged Architecture Definitions.
//!
//! Defines constants and structures for the RISC-V Privileged Specification, including
//! Control and Status Registers (CSRs), Trap Causes, and System Opcodes.
//!
//! # Modules
//!
//! - `cause`: Exception and Interrupt cause codes.
//! - `opcodes`: System instruction opcodes (ECALL, EBREAK, xRET).

/// Exception and interrupt cause code definitions.
pub mod cause;

/// System instruction opcodes (ECALL, EBREAK, xRET, FENCE).
pub mod opcodes;

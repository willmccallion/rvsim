//! # CSR Unit Tests
//!
//! This module serves as the entry point for unit tests related to the RISC-V
//! Control and Status Registers (CSRs). It organizes tests into logical groups
//! covering access control, performance counters, and trap setup.

pub mod access_control;
pub mod cbo_gates;
pub mod counters;
pub mod cpu_csr_operations;
pub mod misa_string;
pub mod trap_setup;

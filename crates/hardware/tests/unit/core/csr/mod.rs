//! # CSR Unit Tests
//!
//! This module serves as the entry point for unit tests related to the RISC-V
//! Control and Status Registers (CSRs). It organizes tests into logical groups
//! covering access control, performance counters, and trap setup.

/// Unit tests for RISC-V Control and Status Register (CSR) access control.
///
/// This module verifies the initialization, read/write logic, and architectural
/// constraints for both Machine-mode and Supervisor-mode registers.
pub mod access_control;

/// Unit tests for RISC-V Control and Status Register (CSR) counters.
///
/// This module verifies the behavior of performance-related counters, including
/// incrementing logic, handling of maximum values, and overflow wrapping.
pub mod counters;

/// Unit tests for trap-related Control and Status Register (CSR) configurations.
///
/// This module verifies the logic for trap delegation, vector modes,
/// and interrupt enable/pending bits within the RISC-V architecture.
pub mod trap_setup;

/// Unit tests for CPU CSR read/write operations.
///
/// This module verifies the CPU-level CSR operations including side effects
/// like TLB flushes, interrupt inhibition, and MSTATUS/SSTATUS synchronization.
pub mod cpu_csr_operations;

/// Unit tests for the Zicboz / Zicbom CSR gate helpers.
pub mod cbo_gates;
pub mod misa_string;

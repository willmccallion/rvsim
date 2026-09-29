//! RISC-V architecture-specific components (CSRs, register files, modes, traps).

/// Control and Status Register (CSR) definitions and access logic.
pub mod csr;

/// Floating-Point Register file implementation.
pub mod fpr;

/// General-Purpose Register file implementation.
pub mod gpr;

/// Trap handling and exception processing.
pub mod trap;

/// Architectural Vector Register File (RVV 1.0).
pub mod vpr;

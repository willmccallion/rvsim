//! RISC-V architecture-specific components.
//!
//! This module contains the implementation of core RISC-V architectural elements.
//! It includes the following modules:
//! 1. **CSRs:** Control and Status Register definitions and access logic.
//! 2. **FPRs:** Floating-Point Register file implementation.
//! 3. **GPRs:** General-Purpose Register file implementation.
//! 4. **Modes:** Privilege mode definitions and transitions.
//! 5. **Traps:** Trap handling and exception processing utilities.

/// Control and Status Register (CSR) definitions and access logic.
pub mod csr;

/// Floating-Point Register file implementation.
pub mod fpr;

/// General-Purpose Register file implementation.
pub mod gpr;

/// Privilege mode definitions and transitions.
pub mod mode;

/// Trap handling and exception processing.
pub mod trap;

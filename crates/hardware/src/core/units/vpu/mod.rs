//! Vector Processing Unit (VPU).
//!
//! This module implements the RISC-V Vector Extension (RVV 1.0) execution units,
//! including types, CSR handling, and vector arithmetic.

/// Vector extension types, newtypes, and vtype CSR parsing.
pub mod types;

/// vsetvl/vsetvli/vsetivli execution logic.
pub mod vsetvl;

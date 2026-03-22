//! Vector Processing Unit (VPU).
//!
//! This module implements the RISC-V Vector Extension (RVV 1.0) execution units,
//! including types, CSR handling, and vector arithmetic.

/// Vector extension types, newtypes, and vtype CSR parsing.
pub mod types;

/// vsetvl/vsetvli/vsetivli execution logic.
pub mod vsetvl;

/// Vector integer ALU: element-wise arithmetic, comparison, and fixed-point.
pub mod alu;

/// Vector instruction execution dispatch (bridges pipeline to VPU).
pub mod execute;

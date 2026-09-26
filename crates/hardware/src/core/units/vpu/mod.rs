//! Vector Processing Unit (VPU).
//!
//! This module implements the RISC-V Vector Extension (RVV 1.0) execution units,
//! including types, CSR handling, and vector arithmetic.

pub mod types;

pub mod vsetvl;

pub mod alu;

pub mod fpu;

pub mod execute;

pub mod mask;

pub mod mem;

pub mod permute;

pub mod reduction;

pub mod regfile;

pub mod shadow;

pub mod lane_model;

pub mod chaining;

pub mod crypto;

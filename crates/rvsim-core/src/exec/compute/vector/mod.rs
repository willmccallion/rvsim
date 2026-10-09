//! Vector Processing Unit (VPU).
//!
//! What RISC-V Vector Extension (RVV 1.0) instructions compute: arithmetic,
//! masks, permutes, reductions, crypto, and memory address generation.

pub mod vsetvl;

pub mod agnostic;

pub mod alu;

pub mod context;

pub mod fpu;

pub mod execute;

pub mod mask;

pub mod mem;

pub mod permute;

pub mod reduction;

pub mod regfile;

pub mod shadow;

pub mod crypto;

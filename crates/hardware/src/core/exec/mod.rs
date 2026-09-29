//! Instruction semantics shared by every engine that runs instructions.
//!
//! The pipelines and the atomic core decode, execute and retire through
//! these functions, so what an instruction does is defined once and only
//! when and how fast it happens differs between engines.

/// Cache-block operations (Zicbom, Zicboz): which may run, and what they do.
pub mod cbo;

/// Instruction decoding.
pub mod decode;

/// What executing an instruction computes and decides.
pub mod execute;

/// A decoded instruction with its operand values.
pub mod inst;

/// What memory instructions compute from the data they access.
pub mod memory;

/// Control signals an instruction decodes to.
pub mod signals;

/// What vector configuration instructions establish.
pub mod vector;

//! Instruction semantics shared by every engine that runs instructions.
//!
//! The pipelines and the atomic core decode, execute and retire through
//! these functions, so what an instruction does is defined once and only
//! when and how fast it happens differs between engines.

/// Control signals an instruction decodes to.
pub mod signals;

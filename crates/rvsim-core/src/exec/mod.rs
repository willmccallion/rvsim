//! Instruction semantics shared by every engine that runs instructions.
//!
//! The in-order and out-of-order pipelines decode, execute and retire
//! through these functions, so what an instruction does is defined once and
//! only when and how fast it happens differs between them.

pub mod state;

pub mod compute;

pub mod cbo;

pub mod decode;

pub mod execute;

pub mod inst;

pub mod memory;

pub mod retire;

pub mod signals;

pub mod vector;

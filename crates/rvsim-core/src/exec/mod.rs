//! Instruction semantics shared by every engine that runs instructions.
//!
//! The pipelines and the atomic core decode, execute and retire through
//! these functions, so what an instruction does is defined once and only
//! when and how fast it happens differs between engines.

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

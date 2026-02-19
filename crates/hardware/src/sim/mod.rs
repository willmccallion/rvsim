//! Simulation utilities, program loading, and the top-level `Simulator`.
//!
//! Provides utilities for loading binaries into memory, setting up
//! the initial system state, and the `Simulator` struct that owns
//! both the CPU and the pipeline.

pub mod loader;
pub mod simulator;

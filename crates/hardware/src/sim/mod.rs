//! Simulation utilities, program loading, and the top-level `Simulator`.
//!
//! Provides utilities for loading binaries into memory, setting up
//! the initial system state, and the `Simulator` struct that owns
//! the simulated chip and the bench-side state.

pub mod components;
pub mod dtb;
pub mod events;
pub mod handle;
pub mod loader;
pub mod packet;
pub mod per_hart_debug;
pub mod simulator;
pub mod state;
pub mod stats;
pub mod topology;

pub use self::state::{CoreCtx, SharedState, SimState};

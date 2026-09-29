//! Simulation: builds the system, drives it cycle by cycle, and reports.
//!
//! The `Simulator` owns the simulated chip and the bench-side state; this
//! layer also loads programs, builds the device tree, saves and restores
//! checkpoints, and collects statistics.

pub mod checkpoint;
pub mod components;
pub mod debug;
pub mod dtb;
pub mod events;
pub mod handle;
pub mod loader;
pub mod packet;
pub mod simulator;
pub mod state;
pub mod stats;
pub mod topology;
pub mod trace;

pub use self::state::{CoreCtx, SharedState, SimState, StageCtx};

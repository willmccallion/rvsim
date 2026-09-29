//! The whole simulated system: building it, running it, saving it.
//!
//! The `Simulator` owns the chip (harts, cores, uncore) and the bench-side
//! state; this layer also loads programs, generates the device tree, and
//! saves and restores checkpoints.

pub mod checkpoint;
pub mod debug;
pub mod dtb;
pub mod loader;
pub mod simulator;
pub mod state;
pub mod topology;

pub use self::state::{CoreCtx, SharedState, SimState, StageCtx};

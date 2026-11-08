//! The whole simulated system: building it, running it, saving it.
//!
//! The `Simulator` owns the chip (harts, cores, uncore) and the bench-side
//! state; this layer also loads programs, generates the device tree, and
//! saves and restores checkpoints.

pub(crate) mod cache_audit;
pub mod checkpoint;
pub(crate) mod coherence_audit;
pub(crate) mod dtb;
pub mod gdb;
pub(crate) mod host_files;
pub mod loader;
pub(crate) mod process;
pub mod simulator;
pub mod snapshot;
pub(crate) mod state;

pub use self::coherence_audit::{InclusionFault, Violation, ViolationKind};
pub use self::process::SpaceError;
pub(crate) use self::state::SystemState;
pub use crate::sim::components::CacheId;
pub use crate::sim::packet::MesiState;
pub use crate::soc::cache::BookkeepingFault;
pub use crate::soc::coherence::protocol::CoreSet;
pub use crate::soc::uncore::{StatsDump, StatsEpoch, TraceControl};

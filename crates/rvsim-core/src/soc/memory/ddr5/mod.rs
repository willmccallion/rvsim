//! DDR5 memory controller with per-bank command state machines.
//!
//! Runs the configuration and JEDEC timing in [`crate::config::ddr5`]
//! against the per-bank / per-rank / per-subchannel dynamic state
//! ([`state`]), the request selection policy ([`scheduler`]), the
//! refresh cadence ([`refresh`]), the ECC scrubber ([`ecc`]), the command
//! state machines ([`controller`]), and their statistics ([`stats`]).

pub mod controller;
pub mod ecc;
pub mod refresh;
pub mod scheduler;
pub mod state;
pub mod stats;
pub use controller::{ClockRatio, Ddr5Controller};
pub use ecc::{EccPolicy, NoEcc, SideBandEcc};
pub use refresh::{AllBank, RankLayout, RefreshPolicy, RefreshTarget, SameBank};
pub use scheduler::{Candidate, Fcfs, FrFcfs, MemScheduler};

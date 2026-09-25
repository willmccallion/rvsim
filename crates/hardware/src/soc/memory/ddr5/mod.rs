//! DDR5 memory controller with per-bank command state machines.
//!
//! Ties together the static configuration ([`config`]), the JEDEC timing
//! table ([`timing`]), the per-bank / per-rank / per-subchannel dynamic
//! state ([`state`]), the request selection policy ([`scheduler`]), the
//! refresh cadence ([`refresh`]), the ECC scrubber ([`ecc`]), and the
//! command state machines ([`controller`]).

pub mod config;
pub mod controller;
pub mod ecc;
pub mod refresh;
pub mod scheduler;
pub mod state;
pub mod timing;

pub use config::{Ddr5Config, PowerDownPolicy};
pub use ecc::{EccKind, EccPolicy, NoEcc, SideBandEcc};
pub use controller::{ClockRatio, Ddr5Controller};
pub use refresh::{AllBank, RankLayout, RefreshKind, RefreshPolicy, RefreshTarget, SameBank};
pub use scheduler::{Candidate, Fcfs, FrFcfs, MemScheduler, SchedulerKind};
pub use timing::{Constraint, Ddr5SpeedBin, Ddr5Timing};

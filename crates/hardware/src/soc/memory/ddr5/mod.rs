//! DDR5 memory controller with per-bank command state machines.
//!
//! Ties together the static configuration ([`config`]), the JEDEC timing
//! table ([`timing`]), the per-bank / per-rank / per-subchannel dynamic
//! state ([`state`]), and the command scheduler ([`controller`]).

pub mod config;
pub mod controller;
pub mod state;
pub mod timing;

pub use config::Ddr5Config;
pub use controller::Ddr5Controller;
pub use timing::Ddr5Timing;

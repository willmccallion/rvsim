//! ECC policies.
//!
//! Side-band ECC (SEC-DED or Chipkill) does not change DRAM timing: the
//! check bits travel with the data. What it adds to the traffic model is
//! patrol scrubbing, a background sweep that reads every line at a fixed
//! rate so latent errors are corrected before they accumulate. An
//! [`EccPolicy`] says whether a controller scrubs and how often.

use std::fmt::Debug;

use crate::soc::memory::ddr5::timing::{Constraint, Ddr5Timing};

/// Decides patrol-scrub cadence.
pub trait EccPolicy: Debug + Send + Sync {
    /// DRAM clocks between consecutive scrub reads, or `None` when the
    /// controller does not scrub.
    fn scrub_interval(&self, timing: &Ddr5Timing) -> Option<u64>;
}

/// No ECC, no scrubbing.
#[derive(Debug, Default)]
pub struct NoEcc;

impl EccPolicy for NoEcc {
    fn scrub_interval(&self, _timing: &Ddr5Timing) -> Option<u64> {
        None
    }
}

/// Side-band ECC with an optional patrol scrubber.
#[derive(Debug)]
pub struct SideBandEcc {
    /// Nanoseconds between scrub reads; `None` disables the scrubber.
    pub patrol_scrub_ns: Option<u64>,
}

impl EccPolicy for SideBandEcc {
    fn scrub_interval(&self, timing: &Ddr5Timing) -> Option<u64> {
        self.patrol_scrub_ns.map(|ns| Constraint::ps(ns * 1000).cycles(timing.data_rate_mts).max(1))
    }
}

/// Which [`EccPolicy`] a controller is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EccKind {
    /// [`NoEcc`].
    #[default]
    None,
    /// Single-error-correct, double-error-detect side-band ECC.
    SecDed {
        /// Nanoseconds between patrol-scrub reads; `None` disables scrubbing.
        patrol_scrub_ns: Option<u64>,
    },
    /// Chipkill (symbol-correcting) side-band ECC.
    ChipKill {
        /// Nanoseconds between patrol-scrub reads; `None` disables scrubbing.
        patrol_scrub_ns: Option<u64>,
    },
}

impl EccKind {
    /// Instantiates the policy.
    #[must_use]
    pub fn build(self) -> Box<dyn EccPolicy> {
        match self {
            Self::None => Box::new(NoEcc),
            Self::SecDed { patrol_scrub_ns } | Self::ChipKill { patrol_scrub_ns } => {
                Box::new(SideBandEcc { patrol_scrub_ns })
            }
        }
    }
}

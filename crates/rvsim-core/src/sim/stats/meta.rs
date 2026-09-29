//! Per-counter metadata: description, unit, and aggregation kind.
//!
//! Every stat registered with [`Stats::register`](super::Stats::register) or
//! [`Stats::derive`](super::Stats::derive) carries a [`Meta`]. Metadata drives
//! auto-summary formatting, aggregation safety (gauges vs accumulators), and
//! unit-aware pretty-printing.

/// The dimensional unit a stat is measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Discrete event count (retired insts, cache hits, mispredicts).
    Events,
    /// A count of simulator cycles (stall cycles, WFI cycles).
    Cycles,
    /// A byte count (bandwidth in bytes, buffer occupancy).
    Bytes,
    /// A pure dimensionless ratio (IPC, CPI, miss rate as fraction).
    Ratio,
    /// A rate per unit time (MIPS, cycles per second).
    Rate,
    /// A ratio expressed as a percentage 0..100 (accuracy%).
    Percent,
}

/// How to interpret a stat over time — whether summing across subjects makes
/// sense, whether it's an instantaneous reading, or a rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A monotonically-increasing counter. Summing across parallel subjects
    /// (e.g. cores) is meaningful.
    Accumulated,
    /// An instantaneous reading. Averaging across subjects makes sense;
    /// summing usually does not.
    Gauge,
    /// A derived rate (IPC, accuracy). Summing is nonsense.
    Rate,
}

/// Metadata attached to every registered counter or derived stat.
#[derive(Clone, Copy, Debug)]
pub struct Meta {
    /// One-line human description.
    pub desc: &'static str,
    /// Physical unit.
    pub unit: Unit,
    /// Aggregation kind.
    pub kind: Kind,
}

impl Meta {
    /// Shorthand for an accumulated event counter.
    #[must_use]
    pub const fn events(desc: &'static str) -> Self {
        Self { desc, unit: Unit::Events, kind: Kind::Accumulated }
    }

    /// Shorthand for an accumulated cycle counter.
    #[must_use]
    pub const fn cycles(desc: &'static str) -> Self {
        Self { desc, unit: Unit::Cycles, kind: Kind::Accumulated }
    }

    /// Shorthand for a derived ratio (IPC/CPI/miss-rate).
    #[must_use]
    pub const fn ratio(desc: &'static str) -> Self {
        Self { desc, unit: Unit::Ratio, kind: Kind::Rate }
    }

    /// Shorthand for a derived percentage (accuracy%).
    #[must_use]
    pub const fn percent(desc: &'static str) -> Self {
        Self { desc, unit: Unit::Percent, kind: Kind::Rate }
    }
}

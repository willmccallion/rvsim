//! The unit each registered stat is measured in, which the summary formats
//! its value by.

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

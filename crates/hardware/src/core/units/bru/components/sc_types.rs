//! Strong types for the Statistical Corrector interface.
//!
//! Prevents parameter mix-ups between TAGE and SC by using newtypes
//! for confidence levels, metadata, and sum values.

/// TAGE confidence level, derived from the provider counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TageConfLevel {
    /// Counter at max: |2*ctr+1| >= 7 (ctr == 3 or -4 for 3-bit).
    High,
    /// Counter at mid: |2*ctr+1| == 5 (ctr == 2 or -3).
    Medium,
    /// Counter at weak: |2*ctr+1| == 1 (ctr == 0 or -1).
    Low,
    /// Everything else.
    None,
}

impl TageConfLevel {
    /// Derive confidence level from a signed TAGE counter value.
    pub const fn from_ctr(ctr: i8) -> Self {
        // Manual abs since i8::abs() is not const-stable.
        let abs_ctr = if ctr < 0 { -(ctr as i32) } else { ctr as i32 };
        let centered = (2 * abs_ctr + 1) as u32;
        match centered {
            c if c >= 7 => Self::High,
            5 => Self::Medium,
            1 => Self::Low,
            _ => Self::None,
        }
    }
}

/// Metadata from TAGE passed to the SC for bias indexing and override decisions.
#[derive(Clone, Copy, Debug)]
pub struct TageScMeta {
    /// Confidence level derived from the TAGE provider counter.
    pub conf: TageConfLevel,
    /// Provider bank index (0 = bimodal base, 1+ = tagged bank).
    pub provider_bank: usize,
    /// Whether an alternate tagged bank matched.
    pub alt_bank_present: bool,
    /// Effective TAGE prediction direction (after `USE_ALT_ON_NA`).
    pub pred_taken: bool,
    /// Effective TAGE counter value (provider or alt after `USE_ALT_ON_NA`).
    /// Used by SC to seed the sum with TAGE confidence.
    pub pred_ctr: i8,
}

/// The SC's sum value -- kept as a distinct type to prevent confusing
/// it with raw counter values or thresholds.
#[derive(Clone, Copy, Debug)]
pub struct ScSum(pub i32);

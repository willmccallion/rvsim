//! The loop predictor.

use serde::Deserialize;

/// Seznec's loop predictor (used by SC-L-TAGE). The defaults are gem5's
/// 64KB TAGE-SC-L loop predictor.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct LoopConfig {
    /// The table holds `2^log_size` entries.
    pub log_size: usize,
    /// In sets of `2^log_assoc` ways.
    pub log_assoc: usize,
    /// Tag bits per entry.
    pub tag_bits: usize,
    /// Iteration-count bits per entry.
    pub iter_bits: usize,
    /// Confidence bits per entry; a saturated counter predicts.
    pub confidence_bits: usize,
    /// Age bits per entry, which replacement consumes.
    pub age_bits: usize,
    /// Bits of the `WITHLOOP` counter that decides whether loop
    /// predictions are used.
    pub use_counter_bits: usize,
    /// Each entry learns whether its loop body is taken or not taken.
    pub use_direction_bit: bool,
    /// The set and tag hash the PC rather than slice it.
    pub use_hashing: bool,
    /// Allocate on one mispredict in four, trying one way.
    pub restrict_allocation: bool,
    /// Iteration count a new entry starts with.
    pub initial_iter: u16,
    /// Age a new entry starts with.
    pub initial_age: u8,
    /// Freeing an entry's count also clears its age.
    pub optional_age_reset: bool,
    /// A long loop predicts before its confidence saturates, once
    /// confidence × iterations exceeds 128 (TAGE-SC-L's rule).
    pub long_loop_confidence: bool,
    /// A correct loop prediction ages its entry up one time in eight even
    /// when TAGE was also right (TAGE-SC-L's rule).
    pub optional_age_increment: bool,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            log_size: 5,
            log_assoc: 2,
            tag_bits: 10,
            iter_bits: 10,
            confidence_bits: 4,
            age_bits: 4,
            use_counter_bits: 7,
            use_direction_bit: true,
            use_hashing: true,
            restrict_allocation: true,
            initial_iter: 0,
            initial_age: 7,
            optional_age_reset: false,
            long_loop_confidence: true,
            optional_age_increment: true,
        }
    }
}

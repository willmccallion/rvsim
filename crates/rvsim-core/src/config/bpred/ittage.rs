//! ITTAGE, the indirect-target predictor.

use serde::{Deserialize, Serialize};

/// Longest ITTAGE history, in history bits: what the global history
/// register holds.
pub const MAX_ITTAGE_HISTORY: usize = 1 << 10;

/// Indirect Target TAGE configuration (used by SC-L-TAGE).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct IttageConfig {
    /// Number of tagged tables
    pub num_banks: usize,

    /// Entries per table
    pub table_size: usize,

    /// History lengths for each bank
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    pub tag_widths: Vec<usize>,

    /// Useful counter reset interval
    pub reset_interval: u32,
}

impl Default for IttageConfig {
    fn default() -> Self {
        Self {
            num_banks: 8,
            table_size: 256,
            history_lengths: vec![4, 8, 16, 32, 64, 128, 256, 512],
            tag_widths: vec![9, 9, 10, 10, 11, 11, 12, 12],
            reset_interval: 256_000,
        }
    }
}

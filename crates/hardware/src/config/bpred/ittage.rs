//! ITTAGE, the indirect-target predictor.

use serde::Deserialize;

/// Indirect Target TAGE configuration (used by SC-L-TAGE).
#[derive(Debug, Clone, Deserialize)]
pub struct IttageConfig {
    /// Number of tagged tables
    #[serde(default = "IttageConfig::default_num_banks")]
    pub num_banks: usize,

    /// Entries per table
    #[serde(default = "IttageConfig::default_table_size")]
    pub table_size: usize,

    /// History lengths for each bank
    #[serde(default = "IttageConfig::default_history_lengths")]
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    #[serde(default = "IttageConfig::default_tag_widths")]
    pub tag_widths: Vec<usize>,

    /// Useful counter reset interval
    #[serde(default = "IttageConfig::default_reset_interval")]
    pub reset_interval: u32,
}

impl Default for IttageConfig {
    fn default() -> Self {
        Self {
            num_banks: Self::default_num_banks(),
            table_size: Self::default_table_size(),
            history_lengths: Self::default_history_lengths(),
            tag_widths: Self::default_tag_widths(),
            reset_interval: Self::default_reset_interval(),
        }
    }
}

impl IttageConfig {
    const fn default_num_banks() -> usize {
        8
    }

    const fn default_table_size() -> usize {
        256
    }

    fn default_history_lengths() -> Vec<usize> {
        vec![4, 8, 16, 32, 64, 128, 256, 512]
    }

    fn default_tag_widths() -> Vec<usize> {
        vec![9, 9, 10, 10, 11, 11, 12, 12]
    }

    const fn default_reset_interval() -> u32 {
        256_000
    }
}

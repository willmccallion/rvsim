//! TAGE and its banking, hashing, history, allocation and update options.

use crate::config::defaults;
use serde::Deserialize;

/// TAGE (Tagged Geometric) predictor configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TageConfig {
    /// Number of tagged tables
    #[serde(default = "TageConfig::default_banks")]
    pub num_banks: usize,

    /// Entries per table
    #[serde(default = "TageConfig::default_table_size")]
    pub table_size: usize,

    /// Useful counter reset interval
    #[serde(default = "TageConfig::default_reset_interval")]
    pub reset_interval: u32,

    /// History lengths for each bank
    #[serde(default = "TageConfig::default_history_lengths")]
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    #[serde(default = "TageConfig::default_tag_widths")]
    pub tag_widths: Vec<usize>,

    /// `USE_ALT_ON_NA` counters. One is `TAGEBase`'s; more are indexed by
    /// the provider's bank group and the alternate's confidence, as
    /// TAGE-SC-L indexes its 16.
    #[serde(default = "TageConfig::default_use_alt_counters")]
    pub use_alt_counters: usize,

    /// Width of each `USE_ALT_ON_NA` counter.
    #[serde(default = "TageConfig::default_use_alt_bits")]
    pub use_alt_bits: u32,

    /// Width of each tagged entry's useful counter.
    #[serde(default = "TageConfig::default_useful_bits")]
    pub useful_bits: u32,

    /// Most entries one misprediction allocates.
    #[serde(default = "TageConfig::default_max_allocations")]
    pub max_allocations: usize,

    /// How a misprediction takes new entries and how useful bits age.
    #[serde(default)]
    pub allocation: TageAllocation,

    /// Which entries a committed branch trains.
    #[serde(default)]
    pub update: TageUpdate,

    /// What each control instruction shifts into the global history.
    #[serde(default)]
    pub history: TageHistoryMode,

    /// Bits of path history the tables hash.
    #[serde(default = "TageConfig::default_path_history_bits")]
    pub path_history_bits: u32,

    /// Bimodal entries, a power of two; `table_size` when absent.
    #[serde(default)]
    pub bimodal_entries: Option<usize>,

    /// `2^bimodal_hysteresis_share_log` bimodal entries share one
    /// hysteresis bit, as `TAGEBase`'s `logRatioBiModalHystEntries`.
    #[serde(default = "TageConfig::default_bimodal_hysteresis_share_log")]
    pub bimodal_hysteresis_share_log: u32,

    /// How the PC enters the table, tag and bimodal hashes.
    #[serde(default)]
    pub hashing: TageHashing,

    /// TAGE-SC-L's table organization; each bank its own table of
    /// `table_size` entries when absent.
    #[serde(default)]
    pub banking: Option<TageBanking>,
}

/// How the PC enters TAGE's hashes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TageHashing {
    /// `TAGEBase`: the PC shifted past its two low bits.
    #[default]
    TageBase,
    /// TAGE-SC-L: the PC unshifted in the tables, and `pc ^ (pc >> 2)` in
    /// the bimodal.
    TageScL,
}

/// TAGE-SC-L's banked tables.
///
/// Banks come in pairs that share a history length; the second of a pair
/// is indexed by the first's index XOR its tag, which makes each pair a
/// 2-way table. Banks before `first_long_bank` share one array of
/// `short_factor * table_size` entries, the rest one of
/// `long_factor * table_size`; each enabled bank takes the next
/// `table_size`-entry slice from a PC- and path-hashed start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TageBanking {
    /// Slices in the short-tag array.
    pub short_factor: usize,
    /// Slices in the long-tag array.
    pub long_factor: usize,
    /// The first bank (0-based) in the long-tag array.
    pub first_long_bank: usize,
    /// Which banks exist (gem5's `noSkip`).
    pub enabled: Vec<bool>,
}

/// What each control instruction shifts into TAGE's global history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TageHistoryMode {
    /// `TAGEBase`: its direction, and one PC bit of path.
    #[default]
    Direction,
    /// TAGE-SC-L: two bits of its PC hashed with its direction (three for
    /// an indirect jump), each with a path bit hashed from its PC.
    PcBits,
}

/// How TAGE allocates entries for a misprediction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TageAllocation {
    /// `TAGEBase`: free entries from one of the next three tables up,
    /// forcing one free when none is; useful bits halve every
    /// `reset_interval` updates.
    #[default]
    TageBase,
    /// CBP-5 TAGE-SC-L: pairs of tables from a randomised start, decaying
    /// strong unuseful entries it passes; useful bits halve once the
    /// allocations that found no free entry outweigh those that did by
    /// `reset_interval`. A branch the final prediction got right allocates
    /// one time in 32.
    Cbp5,
}

/// Which entries TAGE trains on a committed branch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TageUpdate {
    /// `TAGEBase`: the provider, the alternate while the provider is not
    /// useful, and the provider's useful bit when it and the alternate
    /// disagree.
    #[default]
    TageBase,
    /// CBP-5 TAGE-SC-L: the alternate only when a weak provider is wrong;
    /// a provider turning weak, or right beside a saturated right
    /// alternate, loses its useful bit.
    Cbp5,
}

impl Default for TageConfig {
    fn default() -> Self {
        Self {
            num_banks: Self::default_banks(),
            table_size: Self::default_table_size(),
            reset_interval: Self::default_reset_interval(),
            history_lengths: Self::default_history_lengths(),
            tag_widths: Self::default_tag_widths(),
            use_alt_counters: Self::default_use_alt_counters(),
            use_alt_bits: Self::default_use_alt_bits(),
            useful_bits: Self::default_useful_bits(),
            max_allocations: Self::default_max_allocations(),
            allocation: TageAllocation::default(),
            update: TageUpdate::default(),
            history: TageHistoryMode::default(),
            path_history_bits: Self::default_path_history_bits(),
            bimodal_entries: None,
            bimodal_hysteresis_share_log: Self::default_bimodal_hysteresis_share_log(),
            hashing: TageHashing::default(),
            banking: None,
        }
    }
}

impl TageConfig {
    /// Returns the default number of TAGE predictor banks.
    const fn default_banks() -> usize {
        defaults::TAGE_BANKS
    }

    /// Returns the default TAGE predictor table size per bank.
    const fn default_table_size() -> usize {
        defaults::TAGE_TABLE_SIZE
    }

    /// Returns the default TAGE useful counter reset interval.
    const fn default_reset_interval() -> u32 {
        defaults::TAGE_RESET_INTERVAL
    }

    /// Returns the default history lengths for each TAGE bank.
    ///
    /// Geometric progression: [5, 11, 22, 44, 89, 178, 356, 712] (~2× ratio).
    fn default_history_lengths() -> Vec<usize> {
        vec![5, 11, 22, 44, 89, 178, 356, 712]
    }

    /// Returns the default tag widths for each TAGE bank.
    ///
    /// Tag widths increase with history length: [8, 8, 9, 9, 10, 10, 11, 11] bits.
    fn default_tag_widths() -> Vec<usize> {
        vec![8, 8, 9, 9, 10, 10, 11, 11]
    }

    const fn default_use_alt_counters() -> usize {
        1
    }

    const fn default_use_alt_bits() -> u32 {
        4
    }

    const fn default_useful_bits() -> u32 {
        2
    }

    const fn default_max_allocations() -> usize {
        1
    }

    const fn default_path_history_bits() -> u32 {
        16
    }

    const fn default_bimodal_hysteresis_share_log() -> u32 {
        2
    }

    /// The bimodal's entries.
    #[must_use]
    pub fn bimodal_entries(&self) -> usize {
        self.bimodal_entries.unwrap_or(self.table_size)
    }
}

/// Most TAGE banks a configuration may have; 40 covers the 36 logical
/// tables of the 64KB TAGE-SC-L.
pub const MAX_TAGE_BANKS: usize = 40;

/// Longest TAGE history, in history bits.
pub const MAX_TAGE_HISTORY: usize = 1 << 13;

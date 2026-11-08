//! TAGE and its banking, hashing, history, allocation and update options.

use crate::config::checks::check_bank_list;
use crate::config::{ConfigError, defaults};
use serde::{Deserialize, Serialize};

/// TAGE (Tagged Geometric) predictor configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct TageConfig {
    /// Number of tagged tables
    pub num_banks: usize,

    /// Entries per table
    pub table_size: usize,

    /// Useful counter reset interval
    pub reset_interval: u32,

    /// History lengths for each bank
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    pub tag_widths: Vec<usize>,

    /// `USE_ALT_ON_NA` counters. One is `TAGEBase`'s; more are indexed by
    /// the provider's bank group and the alternate's confidence, as
    /// TAGE-SC-L indexes its 16.
    pub use_alt_counters: usize,

    /// Width of each `USE_ALT_ON_NA` counter.
    pub use_alt_bits: u32,

    /// Width of each tagged entry's useful counter.
    pub useful_bits: u32,

    /// Most entries one misprediction allocates.
    pub max_allocations: usize,

    /// How a misprediction takes new entries and how useful bits age.
    pub allocation: TageAllocation,

    /// Which entries a committed branch trains.
    pub update: TageUpdate,

    /// What each control instruction shifts into the global history.
    pub history: TageHistoryMode,

    /// Bits of path history the tables hash.
    pub path_history_bits: u32,

    /// Bimodal entries, a power of two; `table_size` when absent.
    pub bimodal_entries: Option<usize>,

    /// `2^bimodal_hysteresis_share_log` bimodal entries share one
    /// hysteresis bit, as `TAGEBase`'s `logRatioBiModalHystEntries`.
    pub bimodal_hysteresis_share_log: u32,

    /// How the PC enters the table, tag and bimodal hashes.
    pub hashing: TageHashing,

    /// TAGE-SC-L's table organization; each bank its own table of
    /// `table_size` entries when absent.
    pub banking: Option<TageBanking>,
}

/// How the PC enters TAGE's hashes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum TageHistoryMode {
    /// `TAGEBase`: its direction, and one PC bit of path.
    #[default]
    Direction,
    /// TAGE-SC-L: two bits of its PC hashed with its direction (three for
    /// an indirect jump), each with a path bit hashed from its PC.
    PcBits,
}

/// How TAGE allocates entries for a misprediction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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
            num_banks: defaults::TAGE_BANKS,
            table_size: defaults::TAGE_TABLE_SIZE,
            reset_interval: defaults::TAGE_RESET_INTERVAL,
            history_lengths: vec![5, 11, 22, 44, 89, 178, 356, 712],
            tag_widths: vec![8, 8, 9, 9, 10, 10, 11, 11],
            use_alt_counters: 1,
            use_alt_bits: 4,
            useful_bits: 2,
            max_allocations: 1,
            allocation: TageAllocation::default(),
            update: TageUpdate::default(),
            history: TageHistoryMode::default(),
            path_history_bits: 16,
            bimodal_entries: None,
            bimodal_hysteresis_share_log: 2,
            hashing: TageHashing::default(),
            banking: None,
        }
    }
}

impl TageConfig {
    /// The TAGE component of Seznec's 64KB TAGE-SC-L (CBP-5): 36 banked
    /// 1024-entry tables over paired history lengths 6 to 3000, CBP-5
    /// allocation and update with 1-bit useful counters, PC-bit history with
    /// a 27-bit path and an 8K-entry bimodal table.
    #[must_use]
    pub fn tage_sc_l_64kb() -> Self {
        const ENABLED_BANKS: [usize; 21] =
            [1, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 23, 25, 27, 31, 35];
        const BANKS: usize = 36;
        Self {
            num_banks: BANKS,
            table_size: 1024,
            reset_interval: 1024,
            history_lengths: vec![
                6, 6, 9, 9, 12, 12, 18, 18, 26, 26, 37, 37, 54, 54, 78, 78, 112, 112, 161, 161,
                232, 232, 335, 335, 482, 482, 695, 695, 1002, 1002, 1444, 1444, 2081, 2081, 3000,
                3000,
            ],
            tag_widths: [vec![8; 12], vec![12; 24]].concat(),
            use_alt_counters: 16,
            use_alt_bits: 5,
            useful_bits: 1,
            max_allocations: 2,
            allocation: TageAllocation::Cbp5,
            update: TageUpdate::Cbp5,
            history: TageHistoryMode::PcBits,
            path_history_bits: 27,
            bimodal_entries: Some(8192),
            bimodal_hysteresis_share_log: 2,
            hashing: TageHashing::TageScL,
            banking: Some(TageBanking {
                short_factor: 10,
                long_factor: 20,
                first_long_bank: 12,
                enabled: (0..BANKS).map(|bank| ENABLED_BANKS.contains(&bank)).collect(),
            }),
        }
    }

    /// The bimodal's entries.
    #[must_use]
    pub fn bimodal_entries(&self) -> usize {
        self.bimodal_entries.unwrap_or(self.table_size)
    }
}

impl TageConfig {
    /// Checks the banks, tables, histories and counters the predictor can
    /// build.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigError`] found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.use_alt_counters == 0 || !(2..=8).contains(&self.use_alt_bits) {
            return Err(ConfigError::TageUseAlt {
                counters: self.use_alt_counters,
                bits: self.use_alt_bits,
            });
        }
        if self.num_banks > MAX_TAGE_BANKS {
            return Err(ConfigError::TageBanks(self.num_banks));
        }
        check_bank_list("tage", "history_lengths", self.history_lengths.len(), self.num_banks)?;
        check_bank_list("tage", "tag_widths", self.tag_widths.len(), self.num_banks)?;
        if let Some(banking) = &self.banking {
            let fits = banking.enabled.len() == self.num_banks
                && banking.first_long_bank < self.num_banks
                && banking.short_factor > 0
                && banking.long_factor > 0;
            if !fits {
                return Err(ConfigError::TageBanking);
            }
        }
        self.validate_tables()
    }

    /// The tables, histories and allocation counters.
    fn validate_tables(&self) -> Result<(), ConfigError> {
        if !self.table_size.is_power_of_two() {
            return Err(ConfigError::NotPowerOfTwo {
                field: "tage table_size",
                value: self.table_size,
            });
        }
        let bimodal = self.bimodal_entries();
        if !bimodal.is_power_of_two() || (bimodal >> self.bimodal_hysteresis_share_log) == 0 {
            return Err(ConfigError::TageBimodal {
                entries: bimodal,
                share_log: self.bimodal_hysteresis_share_log,
            });
        }
        if !(1..=31).contains(&self.path_history_bits) {
            return Err(ConfigError::TagePathBits(self.path_history_bits));
        }
        if let Some(&length) =
            self.history_lengths.iter().find(|&&length| length > MAX_TAGE_HISTORY)
        {
            return Err(ConfigError::TageHistoryLength(length));
        }
        if !(1..=8).contains(&self.useful_bits) || self.max_allocations == 0 {
            return Err(ConfigError::TageAllocation {
                useful_bits: self.useful_bits,
                max_allocations: self.max_allocations,
            });
        }
        Ok(())
    }
}

/// Most TAGE banks a configuration may have; 40 covers the 36 logical
/// tables of the 64KB TAGE-SC-L.
pub const MAX_TAGE_BANKS: usize = 40;

/// Longest TAGE history, in history bits.
pub const MAX_TAGE_HISTORY: usize = 1 << 13;

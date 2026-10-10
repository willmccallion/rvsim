//! Rocket's branch history table configuration.

use crate::config::defaults;
use serde::Deserialize;
use thiserror::Error;

/// Rocket's BHT (`BHTParams` in rocket-chip's `rocket/BTB.scala`): a table
/// of counters indexed by the fetch PC with a hash of the global history
/// placed above the PC bits.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BhtConfig {
    /// Counters in the table; a power of two.
    #[serde(default = "BhtConfig::default_entries")]
    pub entries: usize,
    /// Bits per counter: 1 or 2.
    #[serde(default = "BhtConfig::default_counter_bits")]
    pub counter_bits: u32,
    /// Bits of global history kept.
    #[serde(default = "BhtConfig::default_history_length")]
    pub history_length: u32,
    /// Bits the history is hashed to, placed above the PC bits of the index.
    #[serde(default = "BhtConfig::default_history_bits")]
    pub history_bits: u32,
}

impl Default for BhtConfig {
    fn default() -> Self {
        Self {
            entries: Self::default_entries(),
            counter_bits: Self::default_counter_bits(),
            history_length: Self::default_history_length(),
            history_bits: Self::default_history_bits(),
        }
    }
}

impl BhtConfig {
    const fn default_entries() -> usize {
        defaults::BHT_ENTRIES
    }

    const fn default_counter_bits() -> u32 {
        defaults::BHT_COUNTER_BITS
    }

    const fn default_history_length() -> u32 {
        defaults::BHT_HISTORY_LENGTH
    }

    const fn default_history_bits() -> u32 {
        defaults::BHT_HISTORY_BITS
    }

    /// Bits of the table index: `log2(entries)`.
    #[must_use]
    pub const fn index_bits(&self) -> u32 {
        self.entries.trailing_zeros()
    }

    /// Checks that the table can be built and indexed.
    ///
    /// # Errors
    ///
    /// The setting that cannot be.
    pub const fn validate(&self) -> Result<(), BhtConfigError> {
        if !self.entries.is_power_of_two() {
            return Err(BhtConfigError::Entries(self.entries));
        }
        if self.counter_bits != 1 && self.counter_bits != 2 {
            return Err(BhtConfigError::CounterBits(self.counter_bits));
        }
        if self.history_length == 0 || self.history_length > 64 {
            return Err(BhtConfigError::HistoryLength(self.history_length));
        }
        if self.history_bits == 0
            || self.history_bits > self.history_length
            || self.history_bits > self.index_bits()
        {
            return Err(BhtConfigError::HistoryBits {
                bits: self.history_bits,
                length: self.history_length,
                index_bits: self.index_bits(),
            });
        }
        Ok(())
    }
}

/// A BHT setting the table cannot be built with.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BhtConfigError {
    /// The entry count must be a power of two for the index.
    #[error("bht entries {0} is not a power of two")]
    Entries(usize),
    /// Rocket's BHT has one- or two-bit counters.
    #[error("bht counter_bits {0} is not 1 or 2")]
    CounterBits(u32),
    /// The history register holds up to 64 bits.
    #[error("bht history_length {0} is not in 1..=64")]
    HistoryLength(u32),
    /// The hashed history must fit both the history and the index.
    #[error(
        "bht history_bits {bits} is not in 1..={length} (history_length) or exceeds the index's {index_bits} bits"
    )]
    HistoryBits {
        /// Configured hashed-history width.
        bits: u32,
        /// Configured history length.
        length: u32,
        /// Bits of the table index.
        index_bits: u32,
    },
}

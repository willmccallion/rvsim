//! The statistical corrector and its GEHL tables.

use serde::Deserialize;

/// One GEHL component of the statistical corrector.
///
/// A table of counters per history length, each indexed by the PC hashed
/// with that many bits of the component's history, and a weight on the
/// component's sum.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct GehlConfig {
    /// History bits each table hashes, longest first; none turns the
    /// component off.
    pub lengths: Vec<u32>,
    /// Each table holds `2^log_entries` counters.
    pub log_entries: u32,
    /// The component's weights start at this value.
    pub weight_init: i8,
}

impl GehlConfig {
    fn new(lengths: &[u32], log_entries: u32, weight_init: i8) -> Self {
        Self { lengths: lengths.to_vec(), log_entries, weight_init }
    }
}

impl Default for GehlConfig {
    fn default() -> Self {
        Self::new(&[], 0, 0)
    }
}

/// A GEHL component over per-branch local histories.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct LocalGehlConfig {
    /// Local histories kept, a power of two.
    pub histories: usize,
    /// A branch's history is at `(pc ^ (pc >> index_shift)) % histories`.
    pub index_shift: u32,
    /// Each update also XORs the branch's `pc & 15` into its history.
    pub mix_pc: bool,
    /// The GEHL tables reading those histories.
    pub gehl: GehlConfig,
}

impl Default for LocalGehlConfig {
    fn default() -> Self {
        Self { histories: 16, index_shift: 2, mix_pc: false, gehl: GehlConfig::default() }
    }
}

/// Seznec's statistical corrector (used by SC-L-TAGE). The defaults are
/// the 64KB TAGE-SC-L's (CBP-5), as gem5's `TAGE_SC_L_64KB` sizes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct ScConfig {
    /// Each of the three bias tables holds `2^log_bias` counters.
    pub log_bias: u32,
    /// Width of the bias and GEHL counters.
    pub counter_bits: u32,
    /// Width of the component weights.
    pub weight_bits: u32,
    /// The bias tables' weights start at this value.
    pub bias_weight_init: i8,
    /// Width of the two choosers between TAGE and the corrector.
    pub chooser_bits: u32,
    /// Width of the global update threshold, kept in eighths.
    pub threshold_bits: u32,
    /// The global update threshold's starting value.
    pub initial_threshold: i32,
    /// The per-PC threshold table holds `2^per_pc_threshold_bits`
    /// counters, and the weight tables half as many index bits.
    pub per_pc_threshold_bits: u32,
    /// Width of the per-PC threshold counters.
    pub per_pc_threshold_width: u32,
    /// The per-PC thresholds' starting value.
    pub initial_per_pc_threshold: i32,
    /// Added to the threshold for each non-negative component weight
    /// (all but the IMLI history component's); 0 keeps it fixed.
    pub threshold_weight_step: i32,
    /// The two shortest-history tables of each GEHL have half the entries.
    pub halve_short_tables: bool,
    /// Width of the IMLI counter, which counts a loop's taken backward
    /// branches; its history table has one entry per count.
    pub imli_counter_bits: u32,
    /// GEHL over the directions of recent conditional branches.
    pub global: GehlConfig,
    /// GEHL over which recent conditional branches were taken backward.
    pub backward: GehlConfig,
    /// GEHL over TAGE's path history.
    pub path: GehlConfig,
    /// GEHLs over local histories.
    pub local: Vec<LocalGehlConfig>,
    /// GEHL over the IMLI counter.
    pub imli: GehlConfig,
    /// GEHL over the outcomes of the branches seen at the current IMLI
    /// count.
    pub imli_history: GehlConfig,
}

/// A statistical corrector setting outside what it can be built with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScConfigError {
    /// A counter width outside the range its storage holds.
    #[error("{field} of {bits} bits is outside {min}..={max}")]
    Width {
        /// The setting.
        field: &'static str,
        /// Its value.
        bits: u32,
        /// The narrowest allowed.
        min: u32,
        /// The widest allowed.
        max: u32,
    },
    /// A GEHL with more tables than its index hash spreads.
    #[error("{component} has {tables} tables; at most {MAX_GEHL_TABLES} are allowed")]
    GehlTables {
        /// The component.
        component: &'static str,
        /// Its table count.
        tables: usize,
    },
    /// A GEHL history length longer than the 64 bits a history holds.
    #[error("{component} reads {length} history bits; at most 64 are allowed")]
    HistoryLength {
        /// The component.
        component: &'static str,
        /// The length.
        length: u32,
    },
    /// More local-history components than the corrector keeps.
    #[error("{0} local components; at most {MAX_LOCAL_HISTORIES} are allowed")]
    LocalComponents(usize),
    /// A local history table whose size is not a power of two.
    #[error("{0} local histories is not a power of two")]
    LocalHistories(usize),
}

/// Most tables in one statistical corrector GEHL component.
pub const MAX_GEHL_TABLES: usize = 8;

/// Most local-history components in the statistical corrector.
pub const MAX_LOCAL_HISTORIES: usize = 4;

impl ScConfig {
    /// Checks the settings the corrector's tables and hashes can hold.
    ///
    /// # Errors
    ///
    /// Returns the first [`ScConfigError`] found.
    pub fn validate(&self) -> Result<(), ScConfigError> {
        let widths = [
            ("counter_bits", self.counter_bits, 2, 8),
            ("weight_bits", self.weight_bits, 2, 8),
            ("chooser_bits", self.chooser_bits, 2, 8),
            ("threshold_bits", self.threshold_bits, 4, 24),
            ("per_pc_threshold_width", self.per_pc_threshold_width, 2, 24),
            ("log_bias", self.log_bias, 2, 24),
            ("per_pc_threshold_bits", self.per_pc_threshold_bits, 0, 24),
            ("imli_counter_bits", self.imli_counter_bits, 1, 16),
        ];
        for (field, bits, min, max) in widths {
            if !(min..=max).contains(&bits) {
                return Err(ScConfigError::Width { field, bits, min, max });
            }
        }
        if self.local.len() > MAX_LOCAL_HISTORIES {
            return Err(ScConfigError::LocalComponents(self.local.len()));
        }
        for local in &self.local {
            if !local.histories.is_power_of_two() {
                return Err(ScConfigError::LocalHistories(local.histories));
            }
            if local.index_shift >= 64 {
                return Err(ScConfigError::Width {
                    field: "local index_shift",
                    bits: local.index_shift,
                    min: 0,
                    max: 63,
                });
            }
        }
        let components = [
            ("global", &self.global),
            ("backward", &self.backward),
            ("path", &self.path),
            ("imli", &self.imli),
            ("imli_history", &self.imli_history),
        ]
        .into_iter()
        .chain(self.local.iter().map(|local| ("local", &local.gehl)));
        for (component, gehl) in components {
            gehl.validate(component, self.halve_short_tables)?;
        }
        Ok(())
    }
}

impl GehlConfig {
    fn validate(
        &self,
        component: &'static str,
        halve_short_tables: bool,
    ) -> Result<(), ScConfigError> {
        if self.lengths.is_empty() {
            return Ok(());
        }
        if self.lengths.len() > MAX_GEHL_TABLES {
            return Err(ScConfigError::GehlTables { component, tables: self.lengths.len() });
        }
        if let Some(&length) = self.lengths.iter().find(|&&length| length > 64) {
            return Err(ScConfigError::HistoryLength { component, length });
        }
        let min = if halve_short_tables { 2 } else { 1 };
        if !(min..=24).contains(&self.log_entries) {
            return Err(ScConfigError::Width {
                field: "GEHL log_entries",
                bits: self.log_entries,
                min,
                max: 24,
            });
        }
        Ok(())
    }
}

impl Default for ScConfig {
    fn default() -> Self {
        Self {
            log_bias: 8,
            counter_bits: 6,
            weight_bits: 6,
            bias_weight_init: 4,
            chooser_bits: 7,
            threshold_bits: 12,
            initial_threshold: 35,
            per_pc_threshold_bits: 6,
            per_pc_threshold_width: 8,
            initial_per_pc_threshold: 0,
            threshold_weight_step: 12,
            halve_short_tables: true,
            imli_counter_bits: 8,
            global: GehlConfig::default(),
            backward: GehlConfig::new(&[40, 24, 10], 10, 7),
            path: GehlConfig::new(&[25, 16, 9], 9, 7),
            local: vec![
                LocalGehlConfig {
                    histories: 256,
                    index_shift: 2,
                    mix_pc: false,
                    gehl: GehlConfig::new(&[11, 6, 3], 10, 7),
                },
                LocalGehlConfig {
                    histories: 16,
                    index_shift: 5,
                    mix_pc: true,
                    gehl: GehlConfig::new(&[16, 11, 6], 9, 7),
                },
                LocalGehlConfig {
                    histories: 16,
                    index_shift: 10,
                    mix_pc: false,
                    gehl: GehlConfig::new(&[9, 4], 10, 7),
                },
            ],
            imli: GehlConfig::new(&[8], 8, 7),
            imli_history: GehlConfig::new(&[10, 4], 9, 0),
        }
    }
}

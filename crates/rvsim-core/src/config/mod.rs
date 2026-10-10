//! Configuration system for the RISC-V simulator.
//!
//! Configuration is supplied via JSON from the Python API (`SimConfig`) or
//! use `Config::default()` for the CLI.

mod bpred;
mod cache;
mod coherence;
pub mod ddr5;
mod defaults;
mod general;
mod memory;
mod pipeline;
mod prefetch;
mod system;

pub use bpred::{
    BranchPredictorKind, GehlConfig, IttageConfig, LocalGehlConfig, LoopConfig, MAX_GEHL_TABLES,
    MAX_LOCAL_HISTORIES, MAX_TAGE_BANKS, MAX_TAGE_HISTORY, PerceptronConfig, ScConfig,
    ScConfigError, TageAllocation, TageBanking, TageConfig, TageHashing, TageHistoryMode,
    TageUpdate, TournamentConfig,
};
pub use cache::{
    CacheConfig, CacheHierarchyConfig, InclusionPolicy, PrefetcherKind, ReplacementPolicyKind,
};
pub use coherence::{
    CoherenceConfig, CoherenceProtocolConfig, HomeAgentConfig, InterconnectConfig,
};
pub use general::{Console, GeneralConfig};
pub use memory::{AddressMappingKind, MemoryConfig, MemoryControllerKind};
pub use pipeline::{
    BackendKind, CsrSquash, FuConfig, MAX_VECTOR_MEM_WIDTH, MemDepPredictorKind, PipelineConfig,
    StoreSetConfig, VecStoreForwarding,
};
pub use prefetch::{LoadPrefetcherConfig, PageBoundary, StorePrefetcherConfig};
pub use system::SystemConfig;

use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use serde::Deserialize;

/// Root configuration structure containing all simulator settings.
///
/// Configuration is supplied by the Python API (`SimConfig.to_dict()` → JSON) or
/// use `Config::default()` for the CLI. No TOML files.
///
/// # Examples
///
/// Creating a default configuration:
///
/// ```
/// use rvsim_core::config::Config;
///
/// let config = Config::default();
/// assert_eq!(config.general.trace_instructions, false);
/// assert_eq!(config.cache.l1_d.size_bytes, 4096);
/// ```
///
/// Deserializing from JSON (typical Python API usage):
///
/// ```
/// use rvsim_core::config::{Config, BranchPredictorKind, PrefetcherKind};
///
/// let json = r#"{
///     "general": {
///         "trace_instructions": true,
///         "start_pc": 2147483648,
///         "direct_mode": true
///     },
///     "system": {
///         "ram_base": 2147483648,
///         "kernel_offset": 2097152
///     },
///     "memory": {
///         "ram_size": 134217728,
///         "controller": "Dram",
///         "t_cas": 14,
///         "t_ras": 14,
///         "t_pre": 14,
///         "tlb_size": 32
///     },
///     "cache": {
///         "l1_d": {
///             "enabled": true,
///             "size_bytes": 32768,
///             "line_bytes": 64,
///             "ways": 4,
///             "latency": 1,
///             "policy": "Lru",
///             "prefetcher": "Stride"
///         },
///         "l1_i": {
///             "enabled": true,
///             "size_bytes": 32768,
///             "line_bytes": 64,
///             "ways": 4,
///             "latency": 1,
///             "policy": "Lru",
///             "prefetcher": "NextLine"
///         },
///         "l2": {
///             "enabled": true,
///             "size_bytes": 131072,
///             "line_bytes": 64,
///             "ways": 8,
///             "latency": 10,
///             "policy": "Lru",
///             "prefetcher": "None"
///         },
///         "l3": {
///             "enabled": false,
///             "size_bytes": 0,
///             "line_bytes": 64,
///             "ways": 1,
///             "latency": 20,
///             "policy": "Lru",
///             "prefetcher": "None"
///         }
///     },
///     "pipeline": {
///         "branch_predictor": "GShare"
///     }
/// }"#;
///
/// let config: Config = serde_json::from_str(json).unwrap();
/// assert_eq!(config.general.trace_instructions, true);
/// assert_eq!(config.cache.l1_d.size_bytes, 32768);
/// assert_eq!(config.cache.l1_d.prefetcher, PrefetcherKind::Stride);
/// assert_eq!(config.pipeline.branch_predictor, BranchPredictorKind::GShare);
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// General simulation settings
    pub general: GeneralConfig,
    /// System memory map and bus parameters
    pub system: SystemConfig,
    /// Main memory configuration
    pub memory: MemoryConfig,
    /// Cache hierarchy configuration
    pub cache: CacheHierarchyConfig,
    /// Coherence fabric between the private caches (used when
    /// `system.hart_count > 1`).
    #[serde(default)]
    pub coherence: CoherenceConfig,
    /// Pipeline and branch predictor configuration
    pub pipeline: PipelineConfig,
    /// ISA capability flags (vector ELEN/Zvfh, future Zvk*/H/Sstc/...).
    #[serde(default)]
    pub isa: crate::isa::config::IsaConfig,
}

/// A configuration the simulator cannot build.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The cache inclusion policy cannot be combined with several harts.
    #[error(
        "inclusion_policy Exclusive is not supported with hart_count > 1: the L2 must be inclusive of its L1s to answer snoops"
    )]
    ExclusiveWithCoherence,
    /// More harts than the coherence structures can track.
    #[error("hart_count {0} exceeds the 64 cores a coherence sharer set can hold")]
    TooManyHarts(usize),
    /// The Simple controller's bandwidth must be positive.
    #[error("simple_bandwidth_gib_s must be a positive number")]
    SimpleBandwidth,
    /// `misa_override` sets V, which needs ELEN = 64.
    #[error("misa_override sets V, but vlen {vlen} / elen {elen} is not the full V extension")]
    VWithoutFullVector {
        /// Configured VLEN in bits.
        vlen: usize,
        /// Configured ELEN in bits.
        elen: usize,
    },
    /// A cache line smaller than the block a cache-block operation acts on,
    /// which every cache level must hold in one line.
    #[error(
        "cache {level} has {line_bytes}-byte lines, smaller than the {CBOZ_BLOCK_SIZE}-byte cache-block-operation block"
    )]
    LineSmallerThanCacheBlock {
        /// The cache level.
        level: &'static str,
        /// Its line size.
        line_bytes: usize,
    },
    /// A vector memory access width that is not a power of two from 8 to
    /// 64 bytes, the widest access within the smallest allowed line.
    #[error("vector_mem_width {0} must be a power of two from 8 to {MAX_VECTOR_MEM_WIDTH} bytes")]
    VectorMemWidth(usize),
    /// More TAGE banks than a prediction record holds.
    #[error("tage num_banks {0} exceeds {MAX_TAGE_BANKS}")]
    TageBanks(usize),
    /// Banking that does not describe the configured banks.
    #[error(
        "tage banking needs one enabled flag per bank, a first_long_bank among them and non-zero factors"
    )]
    TageBanking,
    /// A bimodal that is not a power of two or shares one hysteresis bit
    /// among more entries than it has.
    #[error("tage bimodal_entries {entries} must be a power of two of at least 2^{share_log}")]
    TageBimodal {
        /// Configured entries.
        entries: usize,
        /// Configured sharing.
        share_log: u32,
    },
    /// A path history wider than 31 bits.
    #[error("tage path_history_bits {0} must be in 1..=31")]
    TagePathBits(u32),
    /// A TAGE history longer than the history buffer is sized for.
    #[error("tage history length {0} exceeds {MAX_TAGE_HISTORY}")]
    TageHistoryLength(usize),
    /// Useful counters must fit a `u8` and allocations must take an entry.
    #[error(
        "tage useful_bits {useful_bits} must be in 1..=8 and max_allocations {max_allocations} at least 1"
    )]
    TageAllocation {
        /// Configured useful counter width.
        useful_bits: u32,
        /// Configured allocations per misprediction.
        max_allocations: usize,
    },
    /// `USE_ALT_ON_NA` counters must exist and fit an `i8`.
    #[error("tage use_alt_counters {counters} must be at least 1 and use_alt_bits {bits} in 2..=8")]
    TageUseAlt {
        /// Configured counters.
        counters: usize,
        /// Configured width.
        bits: u32,
    },
    /// A statistical corrector setting outside what it can be built with.
    #[error("sc: {0}")]
    StatCorrector(#[from] ScConfigError),
    /// The BTB's set count must be a power of two for its index hash.
    #[error("btb_size {size} / btb_ways {ways} gives {sets} sets, which is not a power of two")]
    BtbSets {
        /// Configured entries.
        size: usize,
        /// Configured ways.
        ways: usize,
        /// Resulting sets.
        sets: usize,
    },
}

impl Config {
    /// The hart's `misa`: the override when one is given, otherwise
    /// RV64IMAFDC with B, plus V when the vector unit is the full V extension.
    #[must_use]
    pub fn misa(&self) -> crate::isa::misa::Misa {
        self.pipeline
            .misa_override
            .unwrap_or_else(|| crate::isa::misa::Misa::rv64gcb(self.implements_full_v()))
    }

    /// True when the vector unit meets V's minimum: ELEN = 64 (Zve64d);
    /// every [`Vlen`](crate::isa::rvv::Vlen) already satisfies Zvl128b.
    const fn implements_full_v(&self) -> bool {
        self.isa.vector.elen == 64
    }

    /// Checks the combinations the simulator cannot build.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigError`] found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let harts = self.system.hart_count.max(1);
        if harts > 64 {
            return Err(ConfigError::TooManyHarts(harts));
        }
        if harts > 1 && self.cache.inclusion_policy == InclusionPolicy::Exclusive {
            return Err(ConfigError::ExclusiveWithCoherence);
        }
        let ways = self.pipeline.btb_ways.max(1);
        let sets = (self.pipeline.btb_size / ways).max(1);
        if !sets.is_power_of_two() {
            return Err(ConfigError::BtbSets { size: self.pipeline.btb_size, ways, sets });
        }
        if self.misa().has_v() && !self.implements_full_v() {
            return Err(ConfigError::VWithoutFullVector {
                vlen: self.pipeline.vlen.bits(),
                elen: self.isa.vector.elen,
            });
        }
        if self.memory.simple_bandwidth_bytes_per_second().is_none() {
            return Err(ConfigError::SimpleBandwidth);
        }
        let levels = [
            ("l1_d", &self.cache.l1_d),
            ("l1_i", &self.cache.l1_i),
            ("l2", &self.cache.l2),
            ("l3", &self.cache.l3),
        ];
        for (level, cache) in levels {
            let line_bytes = cache.line_bytes;
            if cache.enabled && line_bytes != 0 && (line_bytes as u64) < CBOZ_BLOCK_SIZE {
                return Err(ConfigError::LineSmallerThanCacheBlock { level, line_bytes });
            }
        }
        let tage = &self.pipeline.tage;
        if tage.use_alt_counters == 0 || !(2..=8).contains(&tage.use_alt_bits) {
            return Err(ConfigError::TageUseAlt {
                counters: tage.use_alt_counters,
                bits: tage.use_alt_bits,
            });
        }
        if tage.num_banks > MAX_TAGE_BANKS {
            return Err(ConfigError::TageBanks(tage.num_banks));
        }
        if let Some(banking) = &tage.banking {
            let fits = banking.enabled.len() == tage.num_banks
                && banking.first_long_bank < tage.num_banks
                && banking.short_factor > 0
                && banking.long_factor > 0;
            if !fits {
                return Err(ConfigError::TageBanking);
            }
        }
        let bimodal = tage.bimodal_entries();
        if !bimodal.is_power_of_two() || (bimodal >> tage.bimodal_hysteresis_share_log) == 0 {
            return Err(ConfigError::TageBimodal {
                entries: bimodal,
                share_log: tage.bimodal_hysteresis_share_log,
            });
        }
        if !(1..=31).contains(&tage.path_history_bits) {
            return Err(ConfigError::TagePathBits(tage.path_history_bits));
        }
        if let Some(&length) =
            tage.history_lengths.iter().find(|&&length| length > MAX_TAGE_HISTORY)
        {
            return Err(ConfigError::TageHistoryLength(length));
        }
        if !(1..=8).contains(&tage.useful_bits) || tage.max_allocations == 0 {
            return Err(ConfigError::TageAllocation {
                useful_bits: tage.useful_bits,
                max_allocations: tage.max_allocations,
            });
        }
        self.pipeline.sc.validate()?;
        let vector_mem_width = self.pipeline.vector_mem_width_bytes();
        if !vector_mem_width.is_power_of_two()
            || !(8..=MAX_VECTOR_MEM_WIDTH).contains(&vector_mem_width)
        {
            return Err(ConfigError::VectorMemWidth(vector_mem_width));
        }
        Ok(())
    }
}

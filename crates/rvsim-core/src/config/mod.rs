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
    BhtConfig, BhtConfigError, BranchPredictorKind, GehlConfig, IttageConfig, LocalGehlConfig,
    LoopConfig, MAX_GEHL_TABLES, MAX_LOCAL_HISTORIES, MAX_TAGE_BANKS, MAX_TAGE_HISTORY,
    PerceptronConfig, ScConfig, ScConfigError, TageAllocation, TageBanking, TageConfig,
    TageHashing, TageHistoryMode, TageUpdate, TournamentConfig,
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
use crate::isa::extension::{Extension, Extensions, Isa, ParsedIsa};
use crate::isa::misa::IsaStringError;
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
///         "start_pc": 2147483648
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
    /// The ISA string does not describe a hart the simulator implements.
    #[error("{0}")]
    IsaString(IsaStringError),
    /// The ISA string names a vector register length other than `vlen`.
    #[error("the ISA string names zvl{named}b, but vlen is {configured}")]
    VlenMismatch {
        /// The length the string names.
        named: usize,
        /// The configured VLEN.
        configured: usize,
    },
    /// The ISA string sets V, which needs ELEN = 64.
    #[error("the ISA string sets V, but vlen {vlen} / elen {elen} is not the full V extension")]
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
    /// A BHT setting the table cannot be built with.
    #[error("{0}")]
    Bht(#[from] BhtConfigError),
    /// A stage latency below what the backend's stages can hand on in.
    #[error("pipeline.{field} must be at least {least} on this backend")]
    StageLatency {
        /// The latency field.
        field: &'static str,
        /// The smallest value the backend supports.
        least: u64,
    },
    /// The in-order backend has no rename hold, so a CSR write that steers
    /// execution must squash what was fetched behind it.
    #[error(
        "pipeline.csr_squash Never needs the out-of-order backend's rename hold; use EveryAccess or AffectingWrites"
    )]
    InOrderNeverSquashes,
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
    /// The hart's `misa`: what the ISA string names, otherwise RV64IMAFDC
    /// with B, plus V when the vector unit is the full V extension.
    #[must_use]
    pub fn misa(&self) -> crate::isa::misa::Misa {
        self.isa_set().misa
    }

    /// The ISA string parsed, when the configuration has one.
    ///
    /// # Errors
    ///
    /// [`IsaStringError`] for a string the simulator cannot implement.
    fn parsed_isa(&self) -> Result<Option<ParsedIsa>, IsaStringError> {
        self.isa.string.as_deref().map(Isa::parse).transpose()
    }

    /// True when the vector unit meets V's minimum: ELEN = 64 (Zve64d);
    /// every [`Vlen`](crate::isa::rvv::Vlen) already satisfies Zvl128b.
    const fn implements_full_v(&self) -> bool {
        self.isa.vector.elen == 64
    }

    /// The multi-letter extensions the hart implements: what the ISA
    /// string names, or everything the simulator has, less what the
    /// `zvfh` and `svadu` options leave out.
    #[must_use]
    pub fn extensions(&self) -> Extensions {
        self.isa_set().extensions
    }

    /// The ISA the hart implements, which `misa`, decode and the device
    /// tree follow: the ISA string's when there is one (an unparsable
    /// string, which `validate` refuses, counts as none), else everything.
    #[must_use]
    pub fn isa_set(&self) -> Isa {
        let mut isa = match self.parsed_isa() {
            Ok(Some(parsed)) => parsed.isa,
            Ok(None) | Err(_) => Isa::everything(self.implements_full_v()),
        };
        if !self.isa.vector.zvfh {
            isa.extensions = isa.extensions.without(Extension::Zvfh);
        }
        if self.isa.svadu {
            isa.extensions = isa.extensions.with(Extension::Svadu);
        } else if self.isa.string.is_none() {
            isa.extensions = isa.extensions.without(Extension::Svadu);
        }
        isa
    }

    /// The names of the multi-letter extensions the hart implements, in
    /// ISA-string order, with the vector register length (`zvl128b`) among
    /// the vector ones.
    #[must_use]
    pub fn extension_names(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.extensions().iter().map(|ext| ext.name().to_owned()).collect();
        if self.misa().has_v() {
            let after_vector = names
                .iter()
                .rposition(|name| name.starts_with("zv"))
                .map_or(names.len(), |i| i + 1);
            names.insert(after_vector, format!("zvl{}b", self.pipeline.vlen.bits()));
        }
        names
    }

    /// The ISA string naming the hart's `misa` letters and every extension
    /// it implements, as the device tree advertises it and Linux prints it:
    /// `rv64imafdcbv_zicbom_..._svadu`.
    #[must_use]
    pub fn isa_string(&self) -> String {
        let mut isa = self.misa().isa_string();
        for name in self.extension_names() {
            isa.push('_');
            isa.push_str(&name);
        }
        isa
    }

    /// Checks the combinations the simulator cannot build.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigError`] found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(parsed) = self.parsed_isa().map_err(ConfigError::IsaString)?
            && let Some(named) = parsed.vlen
            && named != self.pipeline.vlen.bits()
        {
            return Err(ConfigError::VlenMismatch { named, configured: self.pipeline.vlen.bits() });
        }
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
        if self.pipeline.fetch_decode_latency == 0 {
            return Err(ConfigError::StageLatency { field: "fetch_decode_latency", least: 1 });
        }
        let least_rename_issue = match self.pipeline.backend {
            BackendKind::InOrder => 1,
            BackendKind::OutOfOrder => 2,
        };
        if self.pipeline.rename_issue_latency < least_rename_issue {
            return Err(ConfigError::StageLatency {
                field: "rename_issue_latency",
                least: least_rename_issue,
            });
        }
        if self.pipeline.backend == BackendKind::InOrder
            && self.pipeline.csr_squash == Some(CsrSquash::Never)
        {
            return Err(ConfigError::InOrderNeverSquashes);
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
        self.pipeline.bht.validate()?;
        let vector_mem_width = self.pipeline.vector_mem_width_bytes();
        if !vector_mem_width.is_power_of_two()
            || !(8..=MAX_VECTOR_MEM_WIDTH).contains(&vector_mem_width)
        {
            return Err(ConfigError::VectorMemWidth(vector_mem_width));
        }
        Ok(())
    }
}

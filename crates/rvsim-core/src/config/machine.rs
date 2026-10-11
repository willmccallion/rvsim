//! The two machines a [`Config`] starts from: the default, which is also the
//! Python API's `Config()`, and the minimal machine every component's own
//! default makes up, which tests start from to isolate one mechanism.

use super::{
    BackendKind, BranchPredictorKind, CacheConfig, CacheHierarchyConfig, CoherenceConfig, Config,
    GeneralConfig, MemoryConfig, PipelineConfig, PrefetcherKind, SystemConfig,
};
use crate::isa::config::IsaConfig;

impl Default for Config {
    /// The Python API's `Config()`: a 4-wide out-of-order core with TAGE,
    /// 32 KiB L1s with next-line and stride prefetchers, a 256 KiB L2 and
    /// fixed-latency memory.
    fn default() -> Self {
        Self {
            memory: MemoryConfig { ram_size: 256 << 20, ..MemoryConfig::default() },
            cache: default_caches(),
            pipeline: default_pipeline(),
            ..Self::minimal()
        }
    }
}

impl Config {
    /// The simplest machine, every component at its own default: a one-wide
    /// in-order core with a static predictor, no caches and fixed-latency
    /// memory. Tests that isolate one mechanism start here and add only
    /// what they exercise.
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            general: GeneralConfig::default(),
            system: SystemConfig::default(),
            memory: MemoryConfig::default(),
            cache: CacheHierarchyConfig::default(),
            coherence: CoherenceConfig::default(),
            pipeline: PipelineConfig::default(),
            isa: IsaConfig::default(),
        }
    }
}

/// The default machine's caches, as the Python API writes them: a level
/// whose prefetcher has no table carries a table size of zero, and the
/// absent L3 a zero geometry.
fn default_caches() -> CacheHierarchyConfig {
    CacheHierarchyConfig {
        l1_i: CacheConfig {
            enabled: true,
            size_bytes: 32 << 10,
            ways: 4,
            prefetcher: PrefetcherKind::NextLine,
            prefetch_table_size: 0,
            ..CacheConfig::default()
        },
        l1_d: CacheConfig {
            enabled: true,
            size_bytes: 32 << 10,
            ways: 4,
            prefetcher: PrefetcherKind::Stride,
            ..CacheConfig::default()
        },
        l2: CacheConfig {
            enabled: true,
            size_bytes: 256 << 10,
            ways: 8,
            latency: 10,
            prefetch_table_size: 0,
            prefetch_degree: 0,
            ..CacheConfig::default()
        },
        l3: CacheConfig {
            size_bytes: 0,
            line_bytes: 0,
            ways: 0,
            latency: 0,
            prefetch_table_size: 0,
            prefetch_degree: 0,
            ..CacheConfig::default()
        },
        ..CacheHierarchyConfig::default()
    }
}

fn default_pipeline() -> PipelineConfig {
    PipelineConfig {
        width: 4,
        branch_predictor: BranchPredictorKind::Tage,
        btb_size: 4096,
        ras_size: 32,
        backend: BackendKind::OutOfOrder,
        rob_size: 128,
        store_buffer_size: 32,
        checkpoint_count: 0,
        ..PipelineConfig::default()
    }
}

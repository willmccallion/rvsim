//! The two machines a [`Config`] starts from: the default, which is also the
//! Python API's `Config()`, and the minimal machine every component's own
//! default makes up, which tests start from to isolate one mechanism.

use super::ddr5::{Ddr5Params, Ddr5SpeedBinName};
use super::defaults::non_zero;
use super::{
    BackendKind, BranchPredictorKind, CacheConfig, CacheHierarchyConfig, CoherenceConfig, Config,
    FuConfig, GeneralConfig, LoadPrefetcherConfig, MemoryConfig, MemoryControllerKind,
    PageBoundary, PipelineConfig, PrefetcherKind, ReplacementPolicyKind, StorePrefetcherConfig,
    SystemConfig, TageConfig,
};
use crate::isa::config::IsaConfig;
use crate::isa::rvv::Vlen;

/// The default machine's clock.
const CLOCK_MHZ: u64 = 4000;

/// The default machine's vector registers.
const VLEN: Vlen = match Vlen::new(256) {
    Ok(vlen) => vlen,
    Err(_) => panic!("the default VLEN must be one V allows"),
};

impl Default for Config {
    /// The Python API's `Config()`: a desktop out-of-order core of the
    /// mid-2020s at 4 GHz, six wide with a 320-entry ROB and the 64KB
    /// TAGE-SC-L, over a 48 KiB L1D, a 1 MiB L2 and a 32 MiB L3 with every
    /// prefetcher, and two channels of DDR5-5600.
    fn default() -> Self {
        Self {
            general: GeneralConfig { clock_hz: CLOCK_MHZ * 1_000_000, ..GeneralConfig::default() },
            system: SystemConfig {
                bus_latency: 1,
                cpu_clock_mhz: CLOCK_MHZ,
                ..SystemConfig::default()
            },
            memory: default_memory(),
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

fn default_memory() -> MemoryConfig {
    MemoryConfig {
        ram_size: 256 << 20,
        controller: MemoryControllerKind::Ddr5,
        ddr5: Ddr5Params::at_speed_bin(Ddr5SpeedBinName::Ddr5_5600B),
        l2_tlb_size: 2048,
        l2_tlb_ways: 8,
        l2_tlb_latency: 7,
        ..MemoryConfig::default()
    }
}

fn default_caches() -> CacheHierarchyConfig {
    CacheHierarchyConfig {
        l1_i: CacheConfig {
            enabled: true,
            size_bytes: 32 << 10,
            ways: 8,
            policy: ReplacementPolicyKind::Plru,
            latency: 1,
            prefetcher: PrefetcherKind::NextLine,
            prefetch_degree: 1,
            mshr_count: non_zero(8),
            ..CacheConfig::default()
        },
        l1_d: CacheConfig {
            enabled: true,
            size_bytes: 48 << 10,
            ways: 12,
            policy: ReplacementPolicyKind::Plru,
            latency: 4,
            prefetcher: PrefetcherKind::NextLine,
            prefetch_degree: 1,
            mshr_count: non_zero(16),
            ..CacheConfig::default()
        },
        l2: CacheConfig {
            enabled: true,
            size_bytes: 1 << 20,
            ways: 8,
            policy: ReplacementPolicyKind::Plru,
            latency: 8,
            prefetcher: PrefetcherKind::Stream,
            prefetch_degree: 4,
            mshr_count: non_zero(32),
            ..CacheConfig::default()
        },
        l3: CacheConfig {
            enabled: true,
            size_bytes: 32 << 20,
            ways: 16,
            policy: ReplacementPolicyKind::Plru,
            latency: 35,
            mshr_count: non_zero(64),
            ..CacheConfig::default()
        },
        wcb_entries: 16,
        load_prefetcher: LoadPrefetcherConfig::Stride {
            table_size: 64,
            l1_lines: 4,
            l2_lines: 16,
            page_boundary: PageBoundary::CrossWithTlb,
        },
        store_prefetcher: StorePrefetcherConfig::stream(),
        ..CacheHierarchyConfig::default()
    }
}

fn default_pipeline() -> PipelineConfig {
    PipelineConfig {
        width: 6,
        branch_predictor: BranchPredictorKind::ScLTage,
        tage: TageConfig::tage_sc_l_64kb(),
        btb_size: 8192,
        btb_ways: 8,
        ras_size: 32,
        backend: BackendKind::OutOfOrder,
        rob_size: 320,
        store_buffer_size: 72,
        issue_queue_size: 128,
        load_queue_size: 128,
        load_ports: 3,
        store_ports: 2,
        prf_gpr_size: 224,
        prf_fpr_size: 192,
        prf_vpr_size: 96,
        fu_config: default_pool(),
        checkpoint_count: 0,
        vlen: VLEN,
        ..PipelineConfig::default()
    }
}

/// The default core's units: four integer ALUs, three memory units and two
/// of each FP and vector arithmetic unit, at a desktop core's latencies.
const fn default_pool() -> FuConfig {
    FuConfig {
        num_int_alu: 4,
        int_alu_latency: 1,
        num_int_mul: 1,
        int_mul_latency: 3,
        num_int_div: 1,
        int_div_latency: 14,
        num_fp_add: 2,
        fp_add_latency: 3,
        num_fp_mul: 2,
        fp_mul_latency: 4,
        num_fp_fma: 2,
        fp_fma_latency: 4,
        num_fp_div_sqrt: 1,
        fp_div_sqrt_latency: 13,
        num_branch: 2,
        branch_latency: 1,
        num_mem: 3,
        mem_latency: 1,
        num_vec_int_alu: 2,
        vec_int_alu_latency: 1,
        num_vec_int_mul: 1,
        vec_int_mul_latency: 3,
        num_vec_int_div: 1,
        vec_int_div_latency: 14,
        num_vec_fp_alu: 2,
        vec_fp_alu_latency: 3,
        num_vec_fp_fma: 2,
        vec_fp_fma_latency: 4,
        num_vec_fp_div_sqrt: 1,
        vec_fp_div_sqrt_latency: 13,
        num_vec_mem: 2,
        vec_mem_latency: 1,
        num_vec_permute: 1,
        vec_permute_latency: 1,
    }
}

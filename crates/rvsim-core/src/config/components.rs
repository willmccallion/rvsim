//! What each configuration component is when nothing sets its fields: the
//! starting point of every builder in the Python API.

use super::ddr5::Ddr5Params;
use super::defaults;
use super::{
    BhtConfig, CacheConfig, CoherenceConfig, FuConfig, HomeAgentConfig, InterconnectConfig,
    IttageConfig, LoadPrefetcherConfig, LoopConfig, MemoryConfig, PerceptronConfig, ScConfig,
    StorePrefetcherConfig, StoreSetConfig, TageConfig, TournamentConfig, VecStoreForwarding,
};
use serde::Serialize;

/// Every configuration component at its own default.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentDefaults {
    /// A cache level.
    pub cache: CacheConfig,
    /// TAGE on its own.
    pub tage: TageConfig,
    /// The 64KB TAGE-SC-L's TAGE, statistical corrector, ITTAGE and loop
    /// predictor.
    pub tage_sc_l: TageScLDefaults,
    /// The perceptron predictor.
    pub perceptron: PerceptronConfig,
    /// The tournament predictor.
    pub tournament: TournamentConfig,
    /// Rocket's BHT.
    pub bht: BhtConfig,
    /// The store-set memory dependence predictor.
    pub store_set: StoreSetConfig,
    /// The structures the in-order backend reads.
    pub in_order: InOrderDefaults,
    /// The load/store unit's stride prefetcher.
    pub load_prefetcher: LoadPrefetcherConfig,
    /// The L1D's store-miss prefetcher.
    pub store_prefetcher: StorePrefetcherConfig,
    /// The memory system, with the Simple and DRAM controllers' timing.
    pub memory: MemoryConfig,
    /// The DDR5 controller.
    pub ddr5: Ddr5Params,
    /// The coherence fabric.
    pub coherence: CoherenceConfig,
    /// The snoop-filter home agent.
    pub snoop_filter: HomeAgentConfig,
    /// An interconnect's hop latency and bandwidth.
    pub interconnect: InterconnectConfig,
}

/// The structures the in-order backend reads, at the sizes an in-order core
/// starts from.
#[derive(Debug, Clone, Serialize)]
pub struct InOrderDefaults {
    /// Instructions in flight from issue to commit.
    pub rob_size: usize,
    /// Committed stores waiting to drain.
    pub store_buffer_size: usize,
    /// Loads that reach memory per cycle.
    pub load_ports: usize,
    /// Stores that reach memory per cycle.
    pub store_ports: usize,
    /// Vector stores in flight.
    pub vec_store_buffer_size: usize,
    /// How a load meets an in-flight vector store.
    pub vec_store_forwarding: VecStoreForwarding,
    /// Its units: a dual-issue core's.
    pub fu_config: FuConfig,
}

/// The parts of the 64KB TAGE-SC-L at their defaults.
#[derive(Debug, Clone, Serialize)]
pub struct TageScLDefaults {
    /// Its TAGE.
    pub tage: TageConfig,
    /// Its statistical corrector.
    pub sc: ScConfig,
    /// Its indirect-target TAGE.
    pub ittage: IttageConfig,
    /// Its loop predictor.
    pub loop_predictor: LoopConfig,
}

impl Default for ComponentDefaults {
    fn default() -> Self {
        Self {
            cache: CacheConfig::default(),
            tage: TageConfig::default(),
            tage_sc_l: TageScLDefaults {
                tage: TageConfig::tage_sc_l_64kb(),
                sc: ScConfig::default(),
                ittage: IttageConfig::default(),
                loop_predictor: LoopConfig::default(),
            },
            perceptron: PerceptronConfig::default(),
            tournament: TournamentConfig::default(),
            bht: BhtConfig::default(),
            store_set: StoreSetConfig::default(),
            in_order: InOrderDefaults {
                rob_size: defaults::ROB_SIZE,
                store_buffer_size: defaults::STORE_BUFFER_SIZE,
                load_ports: defaults::IN_ORDER_LOAD_PORTS,
                store_ports: defaults::STORE_PORTS,
                vec_store_buffer_size: defaults::VEC_STORE_BUFFER_SIZE,
                vec_store_forwarding: VecStoreForwarding::default(),
                fu_config: dual_issue_pool(),
            },
            load_prefetcher: LoadPrefetcherConfig::stride(),
            store_prefetcher: StorePrefetcherConfig::stream(),
            memory: MemoryConfig::default(),
            ddr5: Ddr5Params::default(),
            coherence: CoherenceConfig::default(),
            snoop_filter: HomeAgentConfig::default(),
            interconnect: InterconnectConfig::default(),
        }
    }
}

/// A dual-issue in-order core's units: two integer ALUs and one of every
/// other unit, at the default core's latencies.
const fn dual_issue_pool() -> FuConfig {
    FuConfig {
        num_int_alu: 2,
        int_alu_latency: 1,
        num_int_mul: 1,
        int_mul_latency: 3,
        num_int_div: 1,
        int_div_latency: 14,
        num_fp_add: 1,
        fp_add_latency: 3,
        num_fp_mul: 1,
        fp_mul_latency: 4,
        num_fp_fma: 1,
        fp_fma_latency: 4,
        num_fp_div_sqrt: 1,
        fp_div_sqrt_latency: 13,
        num_branch: 1,
        branch_latency: 1,
        num_mem: 1,
        mem_latency: 1,
        num_vec_int_alu: 1,
        vec_int_alu_latency: 1,
        num_vec_int_mul: 1,
        vec_int_mul_latency: 3,
        num_vec_int_div: 1,
        vec_int_div_latency: 14,
        num_vec_fp_alu: 1,
        vec_fp_alu_latency: 3,
        num_vec_fp_fma: 1,
        vec_fp_fma_latency: 4,
        num_vec_fp_div_sqrt: 1,
        vec_fp_div_sqrt_latency: 13,
        num_vec_mem: 1,
        vec_mem_latency: 1,
        num_vec_permute: 1,
        vec_permute_latency: 1,
    }
}

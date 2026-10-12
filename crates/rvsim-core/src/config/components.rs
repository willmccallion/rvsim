//! What each configuration component is when nothing sets its fields: the
//! starting point of every builder in the Python API.

use super::ddr5::Ddr5Params;
use super::{
    BhtConfig, CacheConfig, CoherenceConfig, FuConfig, HomeAgentConfig, InterconnectConfig,
    IttageConfig, LoadPrefetcherConfig, LoopConfig, MemoryConfig, PerceptronConfig, ScConfig,
    StorePrefetcherConfig, StoreSetConfig, TageConfig, TournamentConfig,
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
    /// The functional unit pool.
    pub fu: FuConfig,
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
            fu: FuConfig::default(),
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

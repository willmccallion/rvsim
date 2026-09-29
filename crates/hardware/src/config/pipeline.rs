//! The pipeline: backend, widths, queues, functional units and memory
//! dependence prediction.

use super::defaults;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::uarch::pipeline::backend::o3::fu_pool::FuConfig;
use crate::uarch::pipeline::engine::BackendType;
use serde::Deserialize;

/// The widest unit-stride vector access: one 64-byte line, the smallest
/// line every cache level must have.
pub const MAX_VECTOR_MEM_WIDTH: usize = CBOZ_BLOCK_SIZE as usize;
use crate::config::{
    BranchPredictorKind, IttageConfig, LoopConfig, PerceptronConfig, ScConfig, TageConfig,
    TournamentConfig,
};

/// Specifies the memory dependence prediction algorithm used to determine
/// whether loads can bypass older unresolved stores at issue time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MemDepPredictorKind {
    /// Blind (conservative) predictor.
    ///
    /// Loads always wait for all older stores to resolve. No speculation,
    /// no violations.
    Blind,
    /// Store-set predictor (Chrysos & Emer 1998), gem5 O3's only predictor.
    ///
    /// Learns load-store dependencies from ordering violations and allows
    /// loads predicted independent to bypass unresolved stores. The default.
    #[default]
    StoreSet,
}

/// Pipeline and branch predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PipelineConfig {
    /// Superscalar width (instructions per cycle)
    #[serde(default = "PipelineConfig::default_width")]
    pub width: usize,

    /// Cycles between commit detecting a trap or interrupt and the pipeline
    /// squashing into its handler.
    #[serde(default = "PipelineConfig::default_trap_latency")]
    pub trap_latency: u64,

    /// Cycles between execute resolving a misprediction, CSR write, fault
    /// or ordering violation and the pipeline squashing into the redirect;
    /// the backend's gem5 value when unset.
    #[serde(default)]
    pub redirect_latency: Option<u64>,

    /// Instructions fetched per cycle; `width` when unset.
    #[serde(default)]
    pub fetch_width: Option<usize>,

    /// Instructions decoded per cycle; `width` when unset.
    #[serde(default)]
    pub decode_width: Option<usize>,

    /// Instructions renamed and dispatched per cycle; `width` when unset.
    #[serde(default)]
    pub rename_width: Option<usize>,

    /// Instructions issued to execute per cycle; `width` when unset.
    #[serde(default)]
    pub issue_width: Option<usize>,

    /// Instructions retired per cycle; `width` when unset.
    #[serde(default)]
    pub commit_width: Option<usize>,

    /// Results written back (waking dependents and completing in the ROB)
    /// per cycle in the out-of-order backend; `width` when unset.
    #[serde(default)]
    pub writeback_width: Option<usize>,

    /// Branch predictor type
    #[serde(default)]
    pub branch_predictor: BranchPredictorKind,

    /// Branch Target Buffer size
    #[serde(default = "PipelineConfig::default_btb_size")]
    pub btb_size: usize,

    /// Branch Target Buffer associativity (ways per set)
    #[serde(default = "PipelineConfig::default_btb_ways")]
    pub btb_ways: usize,

    /// Return Address Stack size
    #[serde(default = "PipelineConfig::default_ras_size")]
    pub ras_size: usize,

    /// `misa` from an ISA string such as `"RV64IMAFDC"`, instead of the
    /// default RV64IMAFDC.
    #[serde(default, deserialize_with = "deserialize_misa")]
    pub misa_override: Option<crate::arch::csr::Misa>,

    /// TAGE predictor configuration
    #[serde(default)]
    pub tage: TageConfig,

    /// Perceptron predictor configuration
    #[serde(default)]
    pub perceptron: PerceptronConfig,

    /// Tournament predictor configuration
    #[serde(default)]
    pub tournament: TournamentConfig,

    /// Statistical Corrector configuration (used by SC-L-TAGE)
    #[serde(default)]
    pub sc: ScConfig,

    /// Indirect Target TAGE configuration (used by SC-L-TAGE)
    #[serde(default)]
    pub ittage: IttageConfig,

    /// Loop predictor configuration (used by SC-L-TAGE)
    #[serde(default)]
    pub loop_predictor: LoopConfig,

    /// Backend type (`InOrder` or `OutOfOrder`)
    #[serde(default)]
    pub backend: BackendType,

    /// Reorder Buffer size
    #[serde(default = "PipelineConfig::default_rob_size")]
    pub rob_size: usize,

    /// Store Buffer size
    #[serde(default = "PipelineConfig::default_store_buffer_size")]
    pub store_buffer_size: usize,

    /// Issue Queue size (for O3 backend)
    #[serde(default = "PipelineConfig::default_issue_queue_size")]
    pub issue_queue_size: usize,

    /// Physical Register File GPR size (O3 backend).
    /// Must satisfy: `prf_gpr_size` >= 32 + `rob_size`.
    #[serde(default = "PipelineConfig::default_prf_gpr_size")]
    pub prf_gpr_size: usize,

    /// Physical Register File FPR size (O3 backend).
    /// Must satisfy: `prf_fpr_size` >= 32 + `rob_size`.
    #[serde(default = "PipelineConfig::default_prf_fpr_size")]
    pub prf_fpr_size: usize,

    /// Load Queue size (O3 backend).
    #[serde(default = "PipelineConfig::default_load_queue_size")]
    pub load_queue_size: usize,

    /// Number of load ports (loads issued per cycle, O3 backend).
    #[serde(default = "PipelineConfig::default_load_ports")]
    pub load_ports: usize,

    /// Number of store ports (stores issued per cycle, O3 backend).
    #[serde(default = "PipelineConfig::default_store_ports")]
    pub store_ports: usize,

    /// Functional unit pool configuration (O3 backend).
    #[serde(default)]
    pub fu_config: FuConfig,

    /// Number of checkpoint slots for O(1) branch recovery (0 = disabled).
    #[serde(default = "PipelineConfig::default_checkpoint_count")]
    pub checkpoint_count: usize,

    /// Memory dependence predictor type
    #[serde(default)]
    pub mem_dep_predictor: MemDepPredictorKind,

    /// Store-set predictor configuration
    #[serde(default)]
    pub store_set: StoreSetConfig,

    /// Vector register width in bits (VLEN). Must be power of 2 in [128, 2048].
    #[serde(default = "PipelineConfig::default_vlen")]
    pub vlen: usize,

    /// Number of vector execution lanes. Defaults to vlen/64 (min 1).
    #[serde(default)]
    pub num_vec_lanes: Option<usize>,

    /// Bytes one unit-stride vector memory access moves: the vector memory
    /// datapath width. Defaults to one register, VLEN/8, up to a line.
    #[serde(default)]
    pub vector_mem_width: Option<usize>,

    /// Vector Physical Register File size (O3 backend).
    #[serde(default = "PipelineConfig::default_prf_vpr_size")]
    pub prf_vpr_size: usize,

    /// Enable vector operation chaining.
    #[serde(default = "PipelineConfig::default_vec_chaining")]
    pub vec_chaining: bool,

    /// Vector Store Buffer capacity (in-flight vec stores). O3 backend only.
    #[serde(default = "PipelineConfig::default_vec_store_buffer_size")]
    pub vec_store_buffer_size: usize,

    /// Forwarding policy for vector-store→load. O3 backend only. Default
    /// `byte_mask` matches BOOM/Apple/Intel/AMD/ARM. `stall` matches Saturn.
    /// `off` always stalls.
    #[serde(default)]
    pub vec_store_forwarding: crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreForwarding,
}

impl PipelineConfig {
    /// Instructions fetched per cycle.
    #[must_use]
    pub const fn fetch_width(&self) -> usize {
        Self::stage_width(self.fetch_width, self.width)
    }

    /// Instructions decoded per cycle.
    #[must_use]
    pub const fn decode_width(&self) -> usize {
        Self::stage_width(self.decode_width, self.width)
    }

    /// Instructions renamed and dispatched per cycle.
    #[must_use]
    pub const fn rename_width(&self) -> usize {
        Self::stage_width(self.rename_width, self.width)
    }

    /// Instructions issued to execute per cycle.
    #[must_use]
    pub const fn issue_width(&self) -> usize {
        Self::stage_width(self.issue_width, self.width)
    }

    /// Instructions retired per cycle.
    #[must_use]
    pub const fn commit_width(&self) -> usize {
        Self::stage_width(self.commit_width, self.width)
    }

    /// Results written back per cycle.
    #[must_use]
    pub const fn writeback_width(&self) -> usize {
        Self::stage_width(self.writeback_width, self.width)
    }

    /// Cycles between execute resolving a redirect and the squash into it.
    #[must_use]
    pub const fn redirect_latency(&self) -> u64 {
        match self.redirect_latency {
            Some(latency) => latency,
            None => match self.backend {
                BackendType::InOrder => defaults::REDIRECT_LATENCY_INORDER,
                BackendType::OutOfOrder => defaults::REDIRECT_LATENCY_O3,
            },
        }
    }

    const fn stage_width(configured: Option<usize>, width: usize) -> usize {
        match configured {
            Some(stage) => stage,
            None => width,
        }
    }

    /// Returns the default pipeline width (instructions per cycle).
    const fn default_width() -> usize {
        defaults::PIPELINE_WIDTH
    }

    /// Returns the default Branch Target Buffer size.
    const fn default_btb_size() -> usize {
        defaults::BTB_SIZE
    }

    /// Returns the default BTB associativity.
    const fn default_btb_ways() -> usize {
        defaults::BTB_WAYS
    }

    /// Returns the default Return Address Stack size.
    const fn default_ras_size() -> usize {
        defaults::RAS_SIZE
    }

    /// Returns the default ROB size.
    const fn default_rob_size() -> usize {
        defaults::ROB_SIZE
    }

    /// Returns the default store buffer size.
    const fn default_store_buffer_size() -> usize {
        defaults::STORE_BUFFER_SIZE
    }

    /// Returns the default issue queue size.
    const fn default_issue_queue_size() -> usize {
        defaults::ISSUE_QUEUE_SIZE
    }

    /// Returns the default PRF GPR size.
    const fn default_prf_gpr_size() -> usize {
        defaults::PRF_GPR_SIZE
    }

    /// Returns the default PRF FPR size.
    const fn default_prf_fpr_size() -> usize {
        defaults::PRF_FPR_SIZE
    }

    /// Returns the default load queue size.
    const fn default_load_queue_size() -> usize {
        defaults::LOAD_QUEUE_SIZE
    }

    /// Returns the default number of load ports.
    /// Vector execution lanes: `num_vec_lanes`, or one per 64 bits of VLEN.
    #[must_use]
    pub fn vector_lanes(&self) -> usize {
        self.num_vec_lanes.unwrap_or_else(|| (self.vlen / 64).max(1))
    }

    /// Bytes one unit-stride vector access moves: `vector_mem_width`, or one
    /// register (VLEN/8) up to the widest access a line allows.
    #[must_use]
    pub fn vector_mem_width_bytes(&self) -> usize {
        self.vector_mem_width.unwrap_or_else(|| (self.vlen / 8).min(MAX_VECTOR_MEM_WIDTH))
    }

    const fn default_load_ports() -> usize {
        defaults::LOAD_PORTS
    }

    /// Returns the default number of store ports.
    const fn default_store_ports() -> usize {
        defaults::STORE_PORTS
    }

    /// Returns the default checkpoint count.
    const fn default_checkpoint_count() -> usize {
        defaults::CHECKPOINT_COUNT
    }

    const fn default_trap_latency() -> u64 {
        defaults::TRAP_LATENCY
    }

    /// Returns the default VLEN.
    const fn default_vlen() -> usize {
        128
    }

    /// Returns the default vector PRF size.
    const fn default_prf_vpr_size() -> usize {
        64
    }

    /// Returns the default vector chaining setting.
    const fn default_vec_chaining() -> bool {
        true
    }

    /// Returns the default Vector Store Buffer size.
    const fn default_vec_store_buffer_size() -> usize {
        defaults::VEC_STORE_BUFFER_SIZE
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            width: defaults::PIPELINE_WIDTH,
            trap_latency: defaults::TRAP_LATENCY,
            redirect_latency: None,
            fetch_width: None,
            decode_width: None,
            rename_width: None,
            issue_width: None,
            commit_width: None,
            writeback_width: None,
            branch_predictor: BranchPredictorKind::default(),
            btb_size: defaults::BTB_SIZE,
            btb_ways: defaults::BTB_WAYS,
            ras_size: defaults::RAS_SIZE,
            misa_override: None,
            tage: TageConfig::default(),
            perceptron: PerceptronConfig::default(),
            tournament: TournamentConfig::default(),
            sc: ScConfig::default(),
            ittage: IttageConfig::default(),
            loop_predictor: LoopConfig::default(),
            backend: BackendType::default(),
            rob_size: defaults::ROB_SIZE,
            store_buffer_size: defaults::STORE_BUFFER_SIZE,
            issue_queue_size: defaults::ISSUE_QUEUE_SIZE,
            prf_gpr_size: defaults::PRF_GPR_SIZE,
            prf_fpr_size: defaults::PRF_FPR_SIZE,
            load_queue_size: defaults::LOAD_QUEUE_SIZE,
            load_ports: defaults::LOAD_PORTS,
            store_ports: defaults::STORE_PORTS,
            fu_config: FuConfig::default(),
            checkpoint_count: defaults::CHECKPOINT_COUNT,
            mem_dep_predictor: MemDepPredictorKind::default(),
            store_set: StoreSetConfig::default(),
            vlen: 128,
            num_vec_lanes: None,
            vector_mem_width: None,
            prf_vpr_size: 64,
            vec_chaining: true,
            vec_store_buffer_size: defaults::VEC_STORE_BUFFER_SIZE,
            vec_store_forwarding:
                crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreForwarding::ByteMask,
        }
    }
}

/// Store-set memory dependence predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct StoreSetConfig {
    /// SSIT (Store Set ID Table) size — indexed by `(pc >> 2) % ssit_size`.
    #[serde(default = "StoreSetConfig::default_ssit_size")]
    pub ssit_size: usize,

    /// LFST (Last Fetched Store Table) size — indexed by store set ID.
    #[serde(default = "StoreSetConfig::default_lfst_size")]
    pub lfst_size: usize,

    /// Loads and stores dispatched between wipes of both tables (0 = never),
    /// so stale learned dependencies do not throttle a program forever.
    #[serde(default = "StoreSetConfig::default_clear_period")]
    pub clear_period: u64,
}

impl Default for StoreSetConfig {
    fn default() -> Self {
        Self {
            ssit_size: Self::default_ssit_size(),
            lfst_size: Self::default_lfst_size(),
            clear_period: Self::default_clear_period(),
        }
    }
}

impl StoreSetConfig {
    /// gem5 O3's `SSITSize`.
    const fn default_ssit_size() -> usize {
        1024
    }

    /// gem5 O3's `LFSTSize`.
    const fn default_lfst_size() -> usize {
        1024
    }

    /// gem5 O3's `store_set_clear_period`.
    const fn default_clear_period() -> u64 {
        250_000
    }
}

fn deserialize_misa<'de, D>(deserializer: D) -> Result<Option<crate::arch::csr::Misa>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|isa| isa.parse().map_err(serde::de::Error::custom))
        .transpose()
}

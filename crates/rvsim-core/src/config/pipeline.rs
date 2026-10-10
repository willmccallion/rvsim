//! The pipeline: backend, widths, queues, functional units and memory
//! dependence prediction.

use super::defaults;
use crate::config::{
    BranchPredictorKind, IttageConfig, LoopConfig, PerceptronConfig, ScConfig, TageConfig,
    TournamentConfig,
};
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::rvv::Vlen;
use serde::Deserialize;

/// The widest unit-stride vector access: one 64-byte line, the smallest
/// line every cache level must have.
pub const MAX_VECTOR_MEM_WIDTH: usize = CBOZ_BLOCK_SIZE as usize;

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
#[serde(deny_unknown_fields)]
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

    /// Cycles from fetch2 writing a bundle to decode reading it: one stage
    /// boundary, two for a front end with a stage between its cache
    /// response and decode (BOOM's F3). At least 1.
    #[serde(default = "PipelineConfig::default_stage_latency")]
    pub fetch_decode_latency: u64,

    /// Cycles from decode writing a bundle to rename reading it; `0` runs
    /// both in one stage, as Rocket's ID decodes and reads the registers
    /// together.
    #[serde(default = "PipelineConfig::default_stage_latency")]
    pub decode_rename_latency: u64,

    /// Cycles from rename writing a bundle to the backend issuing from it:
    /// gem5's two from rename to IEW, Rocket's one from ID to EX. At least 1.
    #[serde(default = "PipelineConfig::default_rename_issue_latency")]
    pub rename_issue_latency: u64,

    /// Which CSR accesses squash the instructions fetched behind them and
    /// refetch; the backend's gem5 behaviour when unset (every access on
    /// the in-order backend, none on the out-of-order one, which holds
    /// rename instead).
    #[serde(default)]
    pub csr_squash: Option<CsrSquash>,

    /// Whether a FENCE squashes the instructions fetched behind it when it
    /// commits, as BOOM's `flush_on_commit` does.
    #[serde(default)]
    pub fence_squash: bool,

    /// Cycles from a load matching a store in the store buffer to its data
    /// reaching writeback, where a load the L1D answers takes the L1D hit
    /// latency. The L1D hit latency when unset, as a core whose forwarding
    /// shares the load pipeline; `1` is gem5's O3 LSQ, whose forwarded load
    /// writes back the cycle after it executes; `0` writes the load back in
    /// the cycle it matches. A forwarded vector span takes at least one
    /// cycle.
    #[serde(default)]
    pub store_forward_latency: Option<u64>,

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
    pub misa_override: Option<crate::isa::misa::Misa>,

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
    pub backend: BackendKind,

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

    /// Reorder-buffer entries commit squashes per cycle after a
    /// misprediction, trap or ordering violation; rename is blocked until
    /// it has finished (gem5's `squashWidth`).
    #[serde(default = "PipelineConfig::default_squash_width")]
    pub squash_width: usize,

    /// Memory dependence predictor type
    #[serde(default)]
    pub mem_dep_predictor: MemDepPredictorKind,

    /// Store-set predictor configuration
    #[serde(default)]
    pub store_set: StoreSetConfig,

    /// Vector register width in bits (VLEN), a power of 2 in [128, 2048].
    #[serde(default)]
    pub vlen: Vlen,

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
    pub vec_store_forwarding: crate::config::VecStoreForwarding,
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
                BackendKind::InOrder => defaults::REDIRECT_LATENCY_INORDER,
                BackendKind::OutOfOrder => defaults::REDIRECT_LATENCY_O3,
            },
        }
    }

    /// Which CSR accesses squash the instructions fetched behind them.
    #[must_use]
    pub const fn csr_squash(&self) -> CsrSquash {
        match self.csr_squash {
            Some(policy) => policy,
            None => match self.backend {
                BackendKind::InOrder => CsrSquash::EveryAccess,
                BackendKind::OutOfOrder => CsrSquash::Never,
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
        self.num_vec_lanes.unwrap_or_else(|| (self.vlen.bits() / 64).max(1))
    }

    /// Bytes one unit-stride vector access moves: `vector_mem_width`, or one
    /// register (VLEN/8) up to the widest access a line allows.
    #[must_use]
    pub fn vector_mem_width_bytes(&self) -> usize {
        self.vector_mem_width.unwrap_or_else(|| self.vlen.bytes().min(MAX_VECTOR_MEM_WIDTH))
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

    const fn default_squash_width() -> usize {
        defaults::SQUASH_WIDTH
    }

    const fn default_trap_latency() -> u64 {
        defaults::TRAP_LATENCY
    }

    const fn default_stage_latency() -> u64 {
        1
    }

    /// gem5's two cycles from rename to IEW.
    const fn default_rename_issue_latency() -> u64 {
        2
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
            fetch_decode_latency: 1,
            decode_rename_latency: 1,
            rename_issue_latency: 2,
            csr_squash: None,
            fence_squash: false,
            store_forward_latency: None,
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
            backend: BackendKind::default(),
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
            squash_width: defaults::SQUASH_WIDTH,
            mem_dep_predictor: MemDepPredictorKind::default(),
            store_set: StoreSetConfig::default(),
            vlen: Vlen::default(),
            num_vec_lanes: None,
            vector_mem_width: None,
            prf_vpr_size: 64,
            vec_chaining: true,
            vec_store_buffer_size: defaults::VEC_STORE_BUFFER_SIZE,
            vec_store_forwarding: crate::config::VecStoreForwarding::ByteMask,
        }
    }
}

/// Store-set memory dependence predictor configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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

fn deserialize_misa<'de, D>(deserializer: D) -> Result<Option<crate::isa::misa::Misa>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|isa| isa.parse().map_err(serde::de::Error::custom))
        .transpose()
}

/// Configuration for the functional unit pool.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuConfig {
    /// Number of integer ALU units.
    pub num_int_alu: usize,
    /// Latency of integer ALU operations in cycles.
    pub int_alu_latency: u64,
    /// Number of integer multiplier units.
    pub num_int_mul: usize,
    /// Latency of integer multiply operations in cycles.
    pub int_mul_latency: u64,
    /// Number of integer divider units.
    pub num_int_div: usize,
    /// Latency of integer divide operations in cycles.
    pub int_div_latency: u64,
    /// Number of floating-point adder units.
    pub num_fp_add: usize,
    /// Latency of floating-point add operations in cycles.
    pub fp_add_latency: u64,
    /// Number of floating-point multiplier units.
    pub num_fp_mul: usize,
    /// Latency of floating-point multiply operations in cycles.
    pub fp_mul_latency: u64,
    /// Number of floating-point fused multiply-add units.
    pub num_fp_fma: usize,
    /// Latency of floating-point FMA operations in cycles.
    pub fp_fma_latency: u64,
    /// Number of floating-point divide/sqrt units.
    pub num_fp_div_sqrt: usize,
    /// Latency of floating-point divide/sqrt operations in cycles.
    pub fp_div_sqrt_latency: u64,
    /// Number of branch units.
    pub num_branch: usize,
    /// Latency of branch operations in cycles.
    pub branch_latency: u64,
    /// Number of memory (load/store) units.
    pub num_mem: usize,
    /// Latency of memory operations in cycles.
    pub mem_latency: u64,
    /// Number of vector integer ALU units.
    #[serde(default = "default_num_vec_int_alu")]
    pub num_vec_int_alu: usize,
    /// Startup latency of vector integer ALU operations.
    #[serde(default = "default_vec_int_alu_latency")]
    pub vec_int_alu_latency: u64,
    /// Number of vector integer multiplier units.
    #[serde(default = "default_num_vec_int_mul")]
    pub num_vec_int_mul: usize,
    /// Startup latency of vector integer multiply operations.
    #[serde(default = "default_vec_int_mul_latency")]
    pub vec_int_mul_latency: u64,
    /// Number of vector integer divider units.
    #[serde(default = "default_num_vec_int_div")]
    pub num_vec_int_div: usize,
    /// Per-element latency of vector integer divide operations.
    #[serde(default = "default_vec_int_div_latency")]
    pub vec_int_div_latency: u64,
    /// Number of vector FP ALU units.
    #[serde(default = "default_num_vec_fp_alu")]
    pub num_vec_fp_alu: usize,
    /// Startup latency of vector FP ALU operations.
    #[serde(default = "default_vec_fp_alu_latency")]
    pub vec_fp_alu_latency: u64,
    /// Number of vector FP FMA units.
    #[serde(default = "default_num_vec_fp_fma")]
    pub num_vec_fp_fma: usize,
    /// Startup latency of vector FP FMA operations.
    #[serde(default = "default_vec_fp_fma_latency")]
    pub vec_fp_fma_latency: u64,
    /// Number of vector FP div/sqrt units.
    #[serde(default = "default_num_vec_fp_div_sqrt")]
    pub num_vec_fp_div_sqrt: usize,
    /// Per-element latency of vector FP div/sqrt operations.
    #[serde(default = "default_vec_fp_div_sqrt_latency")]
    pub vec_fp_div_sqrt_latency: u64,
    /// Number of vector memory units.
    #[serde(default = "default_num_vec_mem")]
    pub num_vec_mem: usize,
    /// Startup latency of vector memory operations.
    #[serde(default = "default_vec_mem_latency")]
    pub vec_mem_latency: u64,
    /// Number of vector permute units.
    #[serde(default = "default_num_vec_permute")]
    pub num_vec_permute: usize,
    /// Startup latency of vector permute operations.
    #[serde(default = "default_vec_permute_latency")]
    pub vec_permute_latency: u64,
}

impl Default for FuConfig {
    fn default() -> Self {
        Self {
            num_int_alu: 4,
            int_alu_latency: 1,
            num_int_mul: 1,
            int_mul_latency: 3,
            num_int_div: 1,
            int_div_latency: 35,
            num_fp_add: 2,
            fp_add_latency: 4,
            num_fp_mul: 2,
            fp_mul_latency: 5,
            num_fp_fma: 2,
            fp_fma_latency: 5,
            num_fp_div_sqrt: 1,
            fp_div_sqrt_latency: 21,
            num_branch: 2,
            branch_latency: 1,
            num_mem: 2,
            mem_latency: 1,
            num_vec_int_alu: default_num_vec_int_alu(),
            vec_int_alu_latency: default_vec_int_alu_latency(),
            num_vec_int_mul: default_num_vec_int_mul(),
            vec_int_mul_latency: default_vec_int_mul_latency(),
            num_vec_int_div: default_num_vec_int_div(),
            vec_int_div_latency: default_vec_int_div_latency(),
            num_vec_fp_alu: default_num_vec_fp_alu(),
            vec_fp_alu_latency: default_vec_fp_alu_latency(),
            num_vec_fp_fma: default_num_vec_fp_fma(),
            vec_fp_fma_latency: default_vec_fp_fma_latency(),
            num_vec_fp_div_sqrt: default_num_vec_fp_div_sqrt(),
            vec_fp_div_sqrt_latency: default_vec_fp_div_sqrt_latency(),
            num_vec_mem: default_num_vec_mem(),
            vec_mem_latency: default_vec_mem_latency(),
            num_vec_permute: default_num_vec_permute(),
            vec_permute_latency: default_vec_permute_latency(),
        }
    }
}

/// Backend type selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BackendKind {
    /// In-order pipeline (default).
    #[default]
    InOrder,
    /// Out-of-order pipeline (future).
    OutOfOrder,
}

/// Which CSR accesses squash the instructions fetched behind them and
/// refetch, as the modelled core's decoder has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CsrSquash {
    /// Every CSR instruction: gem5's `MinorCPU`, and BOOM, whose decoder
    /// marks each one `flush_on_commit` (`v4/exu/decode.scala`).
    EveryAccess,
    /// A write to a CSR whose value steers execution and never a read:
    /// Rocket's `write_flush` (`rocket/CSR.scala`), which exempts the
    /// scratch, epc, cause and tval CSRs.
    AffectingWrites,
    /// None: gem5's O3, which holds rename after the access instead.
    Never,
}

impl CsrSquash {
    /// Whether an access that writes CSR `written` (`None` for a read)
    /// squashes what follows it.
    #[must_use]
    pub const fn squashes(self, written: Option<u32>) -> bool {
        match self {
            Self::EveryAccess => true,
            Self::AffectingWrites => match written {
                Some(addr) => write_affects_execution(addr),
                None => false,
            },
            Self::Never => false,
        }
    }
}

/// Rocket's `write_flush`: a CSR write flushes unless the CSR, taken as
/// its M-mode counterpart, is mscratch, mepc, mcause or mtval.
const fn write_affects_execution(addr: u32) -> bool {
    let as_machine = addr | 0x300;
    !(as_machine >= 0x340 && as_machine <= 0x343)
}

/// Forwarding policy. Selects how `forward_load` reacts to in-flight vec stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VecStoreForwarding {
    /// Per-line byte-mask forwarding (BOOM/Apple/Intel/AMD/ARM pattern). Default.
    #[default]
    ByteMask,
    /// Saturn pattern: never forward; stall on overlap; miss otherwise.
    Stall,
    /// Most conservative: stall on any older in-flight vec store.
    Off,
}

const fn default_num_vec_int_alu() -> usize {
    1
}

const fn default_vec_int_alu_latency() -> u64 {
    1
}

const fn default_num_vec_int_mul() -> usize {
    1
}

const fn default_vec_int_mul_latency() -> u64 {
    3
}

const fn default_num_vec_int_div() -> usize {
    1
}

const fn default_vec_int_div_latency() -> u64 {
    20
}

const fn default_num_vec_fp_alu() -> usize {
    1
}

const fn default_vec_fp_alu_latency() -> u64 {
    4
}

const fn default_num_vec_fp_fma() -> usize {
    1
}

const fn default_vec_fp_fma_latency() -> u64 {
    5
}

const fn default_num_vec_fp_div_sqrt() -> usize {
    1
}

const fn default_vec_fp_div_sqrt_latency() -> u64 {
    20
}

const fn default_num_vec_mem() -> usize {
    1
}

const fn default_vec_mem_latency() -> u64 {
    1
}

const fn default_num_vec_permute() -> usize {
    1
}

const fn default_vec_permute_latency() -> u64 {
    1
}

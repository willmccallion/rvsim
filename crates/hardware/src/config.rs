//! Configuration system for the RISC-V simulator.
//!
//! This module defines all configuration structures and enums used to parameterize
//! the simulator. It provides:
//! 1. **Defaults:** Baseline hardware constants (RAM, MMIO, cache, branch predictor).
//! 2. **Structures:** Hierarchical config for general, system, memory, cache, and pipeline.
//! 3. **Enums:** Memory controller, replacement policy, prefetcher, and branch predictor types.
//!
//! Configuration is supplied via JSON from the Python API (`SimConfig`) or use `Config::default()` for the CLI.

use serde::Deserialize;

/// Default configuration constants for the simulator.
///
/// These values define the baseline hardware configuration when not
/// explicitly overridden in TOML configuration files.
mod defaults {
    /// Base address of main system RAM (2 GiB).
    ///
    /// This is the physical address where the main memory region begins.
    /// All memory accesses below this address are treated as MMIO.
    pub const RAM_BASE: u64 = 0x8000_0000;

    /// Total size of main system RAM (128 MiB).
    ///
    /// Defines the physical memory limit. Accesses beyond `RAM_BASE + RAM_SIZE`
    /// will trigger a bus fault.
    pub const RAM_SIZE: usize = 128 * 1024 * 1024;

    /// Offset from RAM base where kernel images are loaded (2 MiB).
    ///
    /// This offset ensures the kernel is loaded at a predictable address
    /// while leaving space for bootloaders and initial stack.
    pub const KERNEL_OFFSET: u64 = 0x0020_0000;

    /// Base address of UART 16550-compatible serial port MMIO region.
    pub const UART_BASE: u64 = 0x1000_0000;

    /// Base address of VirtIO block device MMIO region.
    pub const DISK_BASE: u64 = 0x9000_0000;

    /// Base address of CLINT (Core Local Interruptor) timer MMIO region.
    pub const CLINT_BASE: u64 = 0x0200_0000;

    /// Base address of system controller (power/reset) MMIO region.
    pub const SYSCON_BASE: u64 = 0x0010_0000;

    /// System bus width in bytes (8 bytes = 64-bit bus).
    ///
    /// Determines the maximum transfer size per bus transaction.
    pub const BUS_WIDTH: u64 = 8;

    /// System bus access latency in cycles.
    ///
    /// Fixed overhead for all bus transactions regardless of access type.
    pub const BUS_LATENCY: u64 = 4;

    /// CLINT timer divider (mtime increments every N cycles).
    ///
    /// Divides the simulation cycle counter to produce the machine timer value.
    pub const CLINT_DIVIDER: u64 = 10;

    /// CAS (Column Access Strobe) latency in DRAM cycles.
    ///
    /// Time from column address assertion to data availability for reads.
    pub const T_CAS: u64 = 14;

    /// RAS (Row Access Strobe) latency in DRAM cycles.
    ///
    /// Time required to activate a DRAM row before column access.
    pub const T_RAS: u64 = 14;

    /// Precharge latency in DRAM cycles.
    ///
    /// Time required to close an active row before opening a new one.
    pub const T_PRE: u64 = 14;

    /// Row buffer miss penalty in DRAM cycles.
    ///
    /// Additional latency when accessing a different row than the one
    /// currently open in the row buffer.
    pub const ROW_MISS_LATENCY: u64 = 120;

    /// Translation Lookaside Buffer entry count.
    ///
    /// Number of virtual-to-physical address translations cached in the TLB.
    pub const TLB_SIZE: usize = 32;

    /// Default cache size in bytes (4 KiB).
    pub const CACHE_SIZE: usize = 4096;

    /// Default cache line size in bytes (64 bytes).
    ///
    /// Matches typical modern processor cache line sizes and DRAM burst length.
    pub const CACHE_LINE: usize = 64;

    /// Default cache associativity (1 way = direct-mapped).
    pub const CACHE_WAYS: usize = 1;

    /// Default cache access latency in cycles.
    pub const CACHE_LATENCY: u64 = 1;

    /// Default prefetcher pattern table size (64 entries).
    pub const PREFETCH_TABLE_SIZE: usize = 64;

    /// Default prefetch degree (1 line per trigger).
    pub const PREFETCH_DEGREE: usize = 1;

    /// Default pipeline width (1 instruction per cycle).
    pub const PIPELINE_WIDTH: usize = 1;

    /// Default Branch Target Buffer size (256 entries).
    pub const BTB_SIZE: usize = 256;

    /// Default Return Address Stack size (8 entries).
    pub const RAS_SIZE: usize = 8;

    /// Default number of TAGE predictor banks (4 tagged tables).
    pub const TAGE_BANKS: usize = 4;

    /// Default TAGE predictor table size (2048 entries per bank).
    pub const TAGE_TABLE_SIZE: usize = 2048;

    /// Default TAGE loop predictor table size (256 entries).
    pub const TAGE_LOOP_SIZE: usize = 256;

    /// Default TAGE useful counter reset interval (256K branches).
    pub const TAGE_RESET_INTERVAL: u32 = 256_000;

    /// Default Perceptron predictor global history length (32 bits).
    pub const PERCEPTRON_HISTORY: usize = 32;

    /// Default Perceptron predictor table size (log2, 1024 entries).
    pub const PERCEPTRON_TABLE_BITS: usize = 10;

    /// Default Tournament predictor global history table size (log2, 4096 entries).
    pub const TOURNAMENT_GLOBAL_BITS: usize = 12;

    /// Default Tournament predictor local history table size (log2, 1024 entries).
    pub const TOURNAMENT_LOCAL_HIST_BITS: usize = 10;

    /// Default Tournament predictor local prediction table size (log2, 1024 entries).
    pub const TOURNAMENT_LOCAL_PRED_BITS: usize = 10;
}

/// Memory controller implementation types.
///
/// Specifies the type of memory controller used to model main memory
/// access timing and behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MemoryController {
    /// Simple fixed-latency memory controller.
    ///
    /// All memory accesses take a fixed number of cycles regardless
    /// of address patterns or row buffer state.
    #[default]
    Simple,
    /// DRAM controller with row buffer modeling.
    ///
    /// Models DRAM timing including CAS, RAS, precharge latencies
    /// and row buffer hit/miss penalties for more accurate timing.
    #[serde(alias = "DRAM")]
    Dram,
}

/// Cache replacement policy algorithms.
///
/// Specifies the algorithm used to select which cache line to evict
/// when a new line must be installed in a full cache set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ReplacementPolicy {
    /// Least Recently Used replacement policy.
    ///
    /// Evicts the cache line that was accessed least recently.
    #[default]
    #[serde(alias = "Lru")]
    Lru,
    /// Pseudo-LRU (tree-based) replacement policy.
    ///
    /// Approximates LRU using a binary tree structure for lower
    /// hardware overhead while maintaining good performance.
    #[serde(alias = "Plru")]
    Plru,
    /// First In First Out replacement policy.
    ///
    /// Evicts the oldest cache line in the set (round-robin).
    #[serde(alias = "Fifo")]
    Fifo,
    /// Random replacement policy.
    ///
    /// Evicts a randomly selected cache line from the set.
    #[serde(alias = "Random")]
    Random,
    /// Most Recently Used replacement policy.
    ///
    /// Evicts the cache line that was accessed most recently.
    /// Effective for cyclic access patterns larger than the cache.
    #[serde(alias = "Mru")]
    Mru,
}

/// Hardware prefetcher types for cache prefetching.
///
/// Prefetchers predict future memory accesses and fetch data
/// into the cache before it is needed to reduce miss penalties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Prefetcher {
    /// No prefetching enabled.
    #[default]
    None,
    /// Next-line prefetcher.
    ///
    /// Prefetches the next sequential cache line after each access.
    NextLine,
    /// Stride prefetcher.
    ///
    /// Detects stride patterns in memory accesses and prefetches
    /// addresses following the detected stride.
    Stride,
    /// Stream prefetcher.
    ///
    /// Detects sequential stream direction (ascending/descending) and
    /// prefetches multiple lines ahead.
    Stream,
    /// Tagged prefetcher.
    ///
    /// Prefetches on demand misses and on hits to previously prefetched lines.
    Tagged,
}

/// Branch prediction algorithm types.
///
/// Specifies the branch prediction algorithm used to predict
/// branch directions and targets for improved pipeline performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BranchPredictor {
    /// Static branch predictor (always predict not-taken).
    ///
    /// Simple predictor that always predicts branches as not-taken.
    #[default]
    Static,
    /// Global history branch predictor (gshare).
    ///
    /// Uses global branch history to index a pattern history table.
    GShare,
    /// Perceptron-based neural branch predictor.
    ///
    /// Uses a neural network (perceptron) to learn branch patterns.
    Perceptron,
    /// Tagged Geometric History Length predictor.
    ///
    /// Advanced predictor using multiple history lengths with tags.
    #[serde(alias = "TAGE")]
    Tage,
    /// Tournament predictor combining local and global predictors.
    ///
    /// Selects between local and global predictors based on performance.
    Tournament,
}

/// Root configuration structure containing all simulator settings.
///
/// Configuration is supplied by the Python API (SimConfig.to_dict() → JSON) or
/// use Config::default() for the CLI. No TOML files.
///
/// # Examples
///
/// Creating a default configuration:
///
/// ```
/// use riscv_core::config::Config;
///
/// let config = Config::default();
/// assert_eq!(config.general.trace_instructions, false);
/// assert_eq!(config.cache.l1_d.size_bytes, 4096);
/// ```
///
/// Deserializing from JSON (typical Python API usage):
///
/// ```
/// use riscv_core::config::{Config, BranchPredictor, Prefetcher};
///
/// let json = r#"{
///     "general": {
///         "trace_instructions": true,
///         "start_pc": 2147483648,
///         "direct_mode": true
///     },
///     "system": {
///         "ram_base": 2147483648,
///         "ram_size": 134217728,
///         "kernel_offset": 2097152
///     },
///     "memory": {
///         "controller": "Dram",
///         "t_cas": 14,
///         "t_ras": 14,
///         "t_pre": 14,
///         "row_miss_latency": 120,
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
/// assert_eq!(config.cache.l1_d.prefetcher, Prefetcher::Stride);
/// assert_eq!(config.pipeline.branch_predictor, BranchPredictor::GShare);
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// General simulation settings
    pub general: GeneralConfig,
    /// System memory map and bus parameters
    pub system: SystemConfig,
    /// Main memory configuration
    pub memory: MemoryConfig,
    /// Cache hierarchy configuration
    pub cache: CacheHierarchyConfig,
    /// Pipeline and branch predictor configuration
    pub pipeline: PipelineConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            system: SystemConfig::default(),
            memory: MemoryConfig::default(),
            cache: CacheHierarchyConfig::default(),
            pipeline: PipelineConfig::default(),
        }
    }
}

/// General simulation settings and options.
///
/// Contains high-level simulation configuration such as tracing,
/// initial program counter, and direct (bare-metal) execution mode.
#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    /// Enable instruction tracing to stderr and debug output (hang detection, status updates, mode switches)
    #[serde(default)]
    pub trace_instructions: bool,

    /// Initial PC value (defaults to RAM base)
    #[serde(default = "GeneralConfig::default_start_pc")]
    pub start_pc: u64,

    /// Direct execution mode: bare-metal binary, no kernel. Traps cause exit instead of jumping to MTVEC.
    /// Default true so user only needs to change this when running a kernel.
    #[serde(default = "GeneralConfig::default_direct_mode")]
    pub direct_mode: bool,

    /// Initial stack pointer (only used when direct_mode is true). Defaults to ram_base + 16MiB if not set.
    #[serde(default)]
    pub initial_sp: Option<u64>,
}

impl GeneralConfig {
    /// Returns the default starting program counter.
    fn default_start_pc() -> u64 {
        defaults::RAM_BASE
    }

    /// Default direct mode to true so bare-metal runs work out of the box.
    fn default_direct_mode() -> bool {
        true
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            trace_instructions: false,
            start_pc: defaults::RAM_BASE,
            direct_mode: true,
            initial_sp: None,
        }
    }
}

/// System memory map and bus configuration.
///
/// Defines memory-mapped I/O base addresses, RAM configuration,
/// and system bus parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct SystemConfig {
    /// UART MMIO base address
    #[serde(default = "SystemConfig::default_uart_base")]
    pub uart_base: u64,

    /// VirtIO disk MMIO base address
    #[serde(default = "SystemConfig::default_disk_base")]
    pub disk_base: u64,

    /// Main RAM base address
    #[serde(default = "SystemConfig::default_ram_base")]
    pub ram_base: u64,

    /// CLINT (timer) MMIO base address
    #[serde(default = "SystemConfig::default_clint_base")]
    pub clint_base: u64,

    /// Syscon (power control) MMIO base address
    #[serde(default = "SystemConfig::default_syscon_base")]
    pub syscon_base: u64,

    /// Kernel load offset from RAM base
    #[serde(default = "SystemConfig::default_kernel_offset")]
    pub kernel_offset: u64,

    /// System bus width in bytes
    #[serde(default = "SystemConfig::default_bus_width")]
    pub bus_width: u64,

    /// System bus latency in cycles
    #[serde(default = "SystemConfig::default_bus_latency")]
    pub bus_latency: u64,

    /// CLINT timer divider (mtime increments every N cycles)
    #[serde(default = "SystemConfig::default_clint_divider")]
    pub clint_divider: u64,

    /// When true, UART output goes to stderr (for visibility when run from Python).
    #[serde(default)]
    pub uart_to_stderr: bool,
}

impl SystemConfig {
    /// Returns the default UART MMIO base address.
    fn default_uart_base() -> u64 {
        defaults::UART_BASE
    }

    /// Returns the default VirtIO disk MMIO base address.
    fn default_disk_base() -> u64 {
        defaults::DISK_BASE
    }

    /// Returns the default RAM base address.
    fn default_ram_base() -> u64 {
        defaults::RAM_BASE
    }

    /// Returns the default CLINT MMIO base address.
    fn default_clint_base() -> u64 {
        defaults::CLINT_BASE
    }

    /// Returns the default system controller MMIO base address.
    fn default_syscon_base() -> u64 {
        defaults::SYSCON_BASE
    }

    /// Returns the default kernel load offset from RAM base.
    fn default_kernel_offset() -> u64 {
        defaults::KERNEL_OFFSET
    }

    /// Returns the default system bus width in bytes.
    fn default_bus_width() -> u64 {
        defaults::BUS_WIDTH
    }

    /// Returns the default system bus latency in cycles.
    fn default_bus_latency() -> u64 {
        defaults::BUS_LATENCY
    }

    /// Returns the default CLINT timer divider value.
    fn default_clint_divider() -> u64 {
        defaults::CLINT_DIVIDER
    }
}

impl Default for SystemConfig {
    /// Creates a default system configuration.
    ///
    /// All MMIO base addresses, bus parameters, and kernel offset are set
    /// to their default values from the `defaults` module.
    fn default() -> Self {
        Self {
            uart_base: defaults::UART_BASE,
            disk_base: defaults::DISK_BASE,
            ram_base: defaults::RAM_BASE,
            clint_base: defaults::CLINT_BASE,
            syscon_base: defaults::SYSCON_BASE,
            kernel_offset: defaults::KERNEL_OFFSET,
            bus_width: defaults::BUS_WIDTH,
            bus_latency: defaults::BUS_LATENCY,
            clint_divider: defaults::CLINT_DIVIDER,
            uart_to_stderr: false,
        }
    }
}

/// Main memory system configuration.
///
/// Specifies RAM size, memory controller type, DRAM timing parameters,
/// and TLB configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    /// RAM size in bytes
    #[serde(default = "MemoryConfig::default_ram_size")]
    pub ram_size: usize,

    /// Memory controller type
    #[serde(default)]
    pub controller: MemoryController,

    /// CAS latency (column access strobe)
    #[serde(default = "MemoryConfig::default_t_cas")]
    pub t_cas: u64,

    /// RAS latency (row access strobe)
    #[serde(default = "MemoryConfig::default_t_ras")]
    pub t_ras: u64,

    /// Precharge latency
    #[serde(default = "MemoryConfig::default_t_pre")]
    pub t_pre: u64,

    /// Row buffer miss penalty
    #[serde(default = "MemoryConfig::default_row_miss")]
    pub row_miss_latency: u64,

    /// TLB entry count
    #[serde(default = "MemoryConfig::default_tlb_size")]
    pub tlb_size: usize,
}

impl MemoryConfig {
    /// Returns the default RAM size in bytes.
    fn default_ram_size() -> usize {
        defaults::RAM_SIZE
    }

    /// Returns the default CAS latency in DRAM cycles.
    fn default_t_cas() -> u64 {
        defaults::T_CAS
    }

    /// Returns the default RAS latency in DRAM cycles.
    fn default_t_ras() -> u64 {
        defaults::T_RAS
    }

    /// Returns the default precharge latency in DRAM cycles.
    fn default_t_pre() -> u64 {
        defaults::T_PRE
    }

    /// Returns the default row buffer miss penalty in DRAM cycles.
    fn default_row_miss() -> u64 {
        defaults::ROW_MISS_LATENCY
    }

    /// Returns the default TLB entry count.
    fn default_tlb_size() -> usize {
        defaults::TLB_SIZE
    }
}

impl Default for MemoryConfig {
    /// Creates a default memory configuration.
    ///
    /// Uses simple memory controller, default DRAM timing parameters,
    /// and standard TLB size.
    fn default() -> Self {
        Self {
            ram_size: defaults::RAM_SIZE,
            controller: MemoryController::default(),
            t_cas: defaults::T_CAS,
            t_ras: defaults::T_RAS,
            t_pre: defaults::T_PRE,
            row_miss_latency: defaults::ROW_MISS_LATENCY,
            tlb_size: defaults::TLB_SIZE,
        }
    }
}

/// Cache hierarchy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct CacheHierarchyConfig {
    /// L1 instruction cache
    pub l1_i: CacheConfig,
    /// L1 data cache
    pub l1_d: CacheConfig,
    /// Unified L2 cache
    pub l2: CacheConfig,
    /// Unified L3 cache (optional)
    pub l3: CacheConfig,
}

impl Default for CacheHierarchyConfig {
    fn default() -> Self {
        Self {
            l1_i: CacheConfig::default(),
            l1_d: CacheConfig::default(),
            l2: CacheConfig::default(),
            l3: CacheConfig::default(),
        }
    }
}

/// Individual cache level configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    /// Enable this cache level
    #[serde(default)]
    pub enabled: bool,

    /// Total cache size in bytes
    #[serde(default = "CacheConfig::default_size")]
    pub size_bytes: usize,

    /// Cache line size in bytes
    #[serde(default = "CacheConfig::default_line")]
    pub line_bytes: usize,

    /// Associativity (number of ways)
    #[serde(default = "CacheConfig::default_ways")]
    pub ways: usize,

    /// Replacement policy
    #[serde(default)]
    pub policy: ReplacementPolicy,

    /// Access latency in cycles
    #[serde(default = "CacheConfig::default_latency")]
    pub latency: u64,

    /// Hardware prefetcher type
    #[serde(default)]
    pub prefetcher: Prefetcher,

    /// Prefetcher table size (for stride prefetcher)
    #[serde(default = "CacheConfig::default_prefetch_table")]
    pub prefetch_table_size: usize,

    /// Prefetch degree (lines to prefetch per trigger)
    #[serde(default = "CacheConfig::default_prefetch_degree")]
    pub prefetch_degree: usize,
}

impl CacheConfig {
    /// Returns the default cache size in bytes.
    fn default_size() -> usize {
        defaults::CACHE_SIZE
    }

    /// Returns the default cache line size in bytes.
    fn default_line() -> usize {
        defaults::CACHE_LINE
    }

    /// Returns the default cache associativity (number of ways).
    fn default_ways() -> usize {
        defaults::CACHE_WAYS
    }

    /// Returns the default cache access latency in cycles.
    fn default_latency() -> u64 {
        defaults::CACHE_LATENCY
    }

    /// Returns the default prefetcher pattern table size.
    fn default_prefetch_table() -> usize {
        defaults::PREFETCH_TABLE_SIZE
    }

    /// Returns the default prefetch degree (lines per trigger).
    fn default_prefetch_degree() -> usize {
        defaults::PREFETCH_DEGREE
    }
}

impl Default for CacheConfig {
    /// Creates a default cache configuration.
    ///
    /// Cache is disabled by default, uses direct-mapped associativity,
    /// LRU replacement, no prefetching, and minimal size.
    fn default() -> Self {
        Self {
            enabled: false,
            size_bytes: defaults::CACHE_SIZE,
            line_bytes: defaults::CACHE_LINE,
            ways: defaults::CACHE_WAYS,
            policy: ReplacementPolicy::default(),
            latency: defaults::CACHE_LATENCY,
            prefetcher: Prefetcher::default(),
            prefetch_table_size: defaults::PREFETCH_TABLE_SIZE,
            prefetch_degree: defaults::PREFETCH_DEGREE,
        }
    }
}

/// Pipeline and branch predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PipelineConfig {
    /// Superscalar width (instructions per cycle)
    #[serde(default = "PipelineConfig::default_width")]
    pub width: usize,

    /// Branch predictor type
    #[serde(default)]
    pub branch_predictor: BranchPredictor,

    /// Branch Target Buffer size
    #[serde(default = "PipelineConfig::default_btb_size")]
    pub btb_size: usize,

    /// Return Address Stack size
    #[serde(default = "PipelineConfig::default_ras_size")]
    pub ras_size: usize,

    /// MISA register override (e.g., "RV64IMAFDC")
    #[serde(default)]
    pub misa_override: Option<String>,

    /// TAGE predictor configuration
    #[serde(default)]
    pub tage: TageConfig,

    /// Perceptron predictor configuration
    #[serde(default)]
    pub perceptron: PerceptronConfig,

    /// Tournament predictor configuration
    #[serde(default)]
    pub tournament: TournamentConfig,
}

impl PipelineConfig {
    /// Returns the default pipeline width (instructions per cycle).
    fn default_width() -> usize {
        defaults::PIPELINE_WIDTH
    }

    /// Returns the default Branch Target Buffer size.
    fn default_btb_size() -> usize {
        defaults::BTB_SIZE
    }

    /// Returns the default Return Address Stack size.
    fn default_ras_size() -> usize {
        defaults::RAS_SIZE
    }
}

impl Default for PipelineConfig {
    /// Creates a default pipeline configuration.
    ///
    /// Uses single-issue width, static branch predictor, and default
    /// sizes for BTB and RAS structures.
    fn default() -> Self {
        Self {
            width: defaults::PIPELINE_WIDTH,
            branch_predictor: BranchPredictor::default(),
            btb_size: defaults::BTB_SIZE,
            ras_size: defaults::RAS_SIZE,
            misa_override: None,
            tage: TageConfig::default(),
            perceptron: PerceptronConfig::default(),
            tournament: TournamentConfig::default(),
        }
    }
}

/// TAGE (Tagged Geometric) predictor configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TageConfig {
    /// Number of tagged tables
    #[serde(default = "TageConfig::default_banks")]
    pub num_banks: usize,

    /// Entries per table
    #[serde(default = "TageConfig::default_table_size")]
    pub table_size: usize,

    /// Loop predictor table size
    #[serde(default = "TageConfig::default_loop_size")]
    pub loop_table_size: usize,

    /// Useful counter reset interval
    #[serde(default = "TageConfig::default_reset_interval")]
    pub reset_interval: u32,

    /// History lengths for each bank
    #[serde(default = "TageConfig::default_history_lengths")]
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    #[serde(default = "TageConfig::default_tag_widths")]
    pub tag_widths: Vec<usize>,
}

impl TageConfig {
    /// Returns the default number of TAGE predictor banks.
    fn default_banks() -> usize {
        defaults::TAGE_BANKS
    }

    /// Returns the default TAGE predictor table size per bank.
    fn default_table_size() -> usize {
        defaults::TAGE_TABLE_SIZE
    }

    /// Returns the default TAGE loop predictor table size.
    fn default_loop_size() -> usize {
        defaults::TAGE_LOOP_SIZE
    }

    /// Returns the default TAGE useful counter reset interval.
    fn default_reset_interval() -> u32 {
        defaults::TAGE_RESET_INTERVAL
    }

    /// Returns the default history lengths for each TAGE bank.
    ///
    /// Provides geometric progression of history lengths: [5, 15, 44, 130].
    fn default_history_lengths() -> Vec<usize> {
        vec![5, 15, 44, 130]
    }

    /// Returns the default tag widths for each TAGE bank.
    ///
    /// Tag widths increase with history length: [9, 9, 10, 10] bits.
    fn default_tag_widths() -> Vec<usize> {
        vec![9, 9, 10, 10]
    }
}

/// Perceptron branch predictor configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PerceptronConfig {
    /// Global history length
    #[serde(default = "PerceptronConfig::default_history")]
    pub history_length: usize,

    /// Log2 of perceptron table size
    #[serde(default = "PerceptronConfig::default_table_bits")]
    pub table_bits: usize,
}

impl PerceptronConfig {
    /// Returns the default Perceptron predictor global history length.
    fn default_history() -> usize {
        defaults::PERCEPTRON_HISTORY
    }

    /// Returns the default Perceptron predictor table size (log2).
    fn default_table_bits() -> usize {
        defaults::PERCEPTRON_TABLE_BITS
    }
}

/// Tournament branch predictor configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TournamentConfig {
    /// Global predictor size (log2)
    #[serde(default = "TournamentConfig::default_global")]
    pub global_size_bits: usize,

    /// Local history table size (log2)
    #[serde(default = "TournamentConfig::default_local_hist")]
    pub local_hist_bits: usize,

    /// Local prediction table size (log2)
    #[serde(default = "TournamentConfig::default_local_pred")]
    pub local_pred_bits: usize,
}

impl TournamentConfig {
    /// Returns the default Tournament predictor global history table size (log2).
    fn default_global() -> usize {
        defaults::TOURNAMENT_GLOBAL_BITS
    }

    /// Returns the default Tournament predictor local history table size (log2).
    fn default_local_hist() -> usize {
        defaults::TOURNAMENT_LOCAL_HIST_BITS
    }

    /// Returns the default Tournament predictor local prediction table size (log2).
    fn default_local_pred() -> usize {
        defaults::TOURNAMENT_LOCAL_PRED_BITS
    }
}

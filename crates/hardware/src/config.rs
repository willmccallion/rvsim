//! Configuration system for the RISC-V simulator.
//!
//! Configuration is supplied via JSON from the Python API (`SimConfig`) or
//! use `Config::default()` for the CLI.

use crate::core::pipeline::backend::o3::fu_pool::FuConfig;
use crate::core::pipeline::engine::BackendType;
use crate::isa::zicboz::CBOZ_BLOCK_SIZE;

/// The widest unit-stride vector access: one 64-byte line, the smallest
/// line every cache level must have.
pub const MAX_VECTOR_MEM_WIDTH: usize = CBOZ_BLOCK_SIZE as usize;
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

    /// Base address of `VirtIO` block device MMIO region.
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

    /// Wall-clock time the RTC reports at cycle zero: 2026-01-01T00:00:00Z,
    /// so every run reads the same clock.
    pub const RTC_EPOCH_SECONDS: u64 = 1_767_225_600;

    /// Core clock in MHz. Fixes the ratio between simulator cycles and
    /// wall-clock time for components with their own clock domain (the DDR5
    /// controller runs at the DRAM command clock). Independent of the CLINT
    /// timebase, which is a functional timer setting.
    pub const CPU_CLOCK_MHZ: u64 = 2400;

    /// Time a device takes to answer a register access, in nanoseconds:
    /// gem5's `BasicPioDevice.pio_latency`.
    pub const DEVICE_LATENCY_NS: u64 = 100;

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

    /// Bandwidth of the Simple memory controller in GiB/s (gem5's
    /// `SimpleMemory` default).
    pub const SIMPLE_BANDWIDTH_GIB_S: f64 = 12.8;

    /// Number of DRAM banks per rank (default 8, typical DDR3/DDR4).
    pub const NUM_BANKS: usize = 8;

    /// Row-to-Row Delay (different bank) in DRAM cycles.
    ///
    /// Minimum time between ACT commands to different banks.
    pub const T_RRD: u64 = 4;

    /// DRAM row (page) size in bytes (default 2 KiB).
    pub const ROW_SIZE_BYTES: usize = 2048;

    /// Refresh Interval in cycles (~7.8μs at 1 GHz).
    ///
    /// Time between successive auto-refresh commands.
    pub const T_REFI: u64 = 7800;

    /// Refresh Cycle time in cycles (~350ns at 1 GHz).
    ///
    /// Duration of a single refresh operation during which all banks
    /// are unavailable.
    pub const T_RFC: u64 = 350;

    /// Entries in each L1 TLB (instruction and data): gem5's RISC-V TLB
    /// size.
    pub const TLB_SIZE: usize = 64;

    /// L1 TLB ways per set; zero for fully associative, as in gem5.
    pub const TLB_WAYS: usize = 0;

    /// Entries in the shared L2 TLB; zero for none, as in gem5.
    pub const L2_TLB_SIZE: usize = 0;

    /// L2 TLB associativity (ways per set).
    pub const L2_TLB_WAYS: usize = 4;

    /// L2 TLB hit latency in cycles.
    pub const L2_TLB_LATENCY: u64 = 4;

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
    /// Default cycles from a fill arriving to its requests being answered
    /// (gem5's stdlib caches' `response_latency`).
    pub const CACHE_RESPONSE_LATENCY: u64 = 1;

    /// Default prefetcher pattern table size (64 entries).
    pub const PREFETCH_TABLE_SIZE: usize = 64;

    /// Default prefetch degree (1 line per trigger).
    pub const PREFETCH_DEGREE: usize = 1;

    /// Default outstanding line fetches per cache level (gem5's classic
    /// caches use 4 for an L1 and 20 for an L2; 8 sits between).
    pub const MSHR_COUNT: usize = 8;
    /// Default writeback buffer entries per cache level.
    pub const WRITE_BUFFERS: usize = 8;
    /// Default requests one MSHR can hold (gem5's `tgts_per_mshr` for an
    /// L1; its L2 uses 12).
    pub const TARGETS_PER_MSHR: usize = 20;

    /// Default pipeline width (1 instruction per cycle).
    pub const PIPELINE_WIDTH: usize = 1;

    /// Default Branch Target Buffer size (256 entries).
    pub const BTB_SIZE: usize = 256;

    /// Default Branch Target Buffer associativity (4-way).
    pub const BTB_WAYS: usize = 4;

    /// Default Return Address Stack size (8 entries).
    pub const RAS_SIZE: usize = 8;

    /// Default number of TAGE predictor banks (8 tagged tables).
    pub const TAGE_BANKS: usize = 8;

    /// Default TAGE predictor table size (2048 entries per bank).
    pub const TAGE_TABLE_SIZE: usize = 2048;

    /// Default Reorder Buffer size (64 entries).
    pub const ROB_SIZE: usize = 64;

    /// Default Store Buffer size (16 entries).
    pub const STORE_BUFFER_SIZE: usize = 16;

    /// Default Vector Store Buffer size (8 in-flight vec stores).
    pub const VEC_STORE_BUFFER_SIZE: usize = 8;

    /// Default Issue Queue size (32 entries) for out-of-order backend.
    pub const ISSUE_QUEUE_SIZE: usize = 32;

    /// Default Load Queue size (32 entries) for out-of-order backend.
    pub const LOAD_QUEUE_SIZE: usize = 32;

    /// Default number of load ports (loads issued per cycle) for out-of-order backend.
    pub const LOAD_PORTS: usize = 2;

    /// Default number of store ports (stores issued per cycle) for out-of-order backend.
    pub const STORE_PORTS: usize = 1;

    /// Default checkpoint count for O(1) branch recovery (32 slots).
    /// Real `OoO` processors (e.g. BOOM, ARM Cortex-A77) typically have 16-64 checkpoint slots.
    pub const CHECKPOINT_COUNT: usize = 32;
    /// Cycles from commit detecting a trap to the squash into its handler
    /// (gem5's O3 `trapLatency`).
    pub const TRAP_LATENCY: u64 = 13;

    /// Cycles from an in-order execute resolving a redirect to fetch
    /// taking it: gem5 `MinorCPU`'s execute-to-fetch1 branch latch.
    pub const REDIRECT_LATENCY_INORDER: u64 = 1;

    /// Cycles from an out-of-order execute resolving a redirect to fetch
    /// taking it: gem5's `iewToCommitDelay` plus `commitToFetchDelay`.
    pub const REDIRECT_LATENCY_O3: u64 = 2;

    /// Default Physical Register File GPR size (256 entries).
    pub const PRF_GPR_SIZE: usize = 256;

    /// Default Physical Register File FPR size (128 entries).
    pub const PRF_FPR_SIZE: usize = 128;

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
    /// DDR5 command-level controller (JEDEC-timed).
    ///
    /// Per-bank command state machines across channels, sub-channels and
    /// ranks, JEDEC timing from a speed bin, FR-FCFS scheduling, refresh,
    /// power-down and ECC scrubbing. Parameters come from
    /// [`MemoryConfig::ddr5`].
    #[serde(alias = "DDR5")]
    Ddr5,
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

/// Cache inclusion policy for multi-level cache hierarchies.
///
/// Controls how evictions at one cache level interact with other levels
/// to maintain coherence within the same core's cache hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum InclusionPolicy {
    /// No Inclusion, Non-Exclusive (default).
    ///
    /// Cache levels operate independently. An eviction at one level does
    /// not affect other levels. This is the simplest policy and matches
    /// the existing behavior.
    #[default]
    #[serde(alias = "NINE")]
    Nine,
    /// Inclusive: L2 is a superset of L1.
    ///
    /// When a line is evicted from L2, the corresponding line in L1 is
    /// back-invalidated to prevent L1 from holding stale data.
    Inclusive,
    /// Exclusive: L1 and L2 hold disjoint sets of lines.
    ///
    /// When a line is evicted from L1, it is installed into L2 (swap policy).
    /// This maximizes effective cache capacity.
    Exclusive,
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
    /// SC-L-TAGE + ITTAGE composed predictor.
    ///
    /// Combines TAGE, Loop, Statistical Corrector, and Indirect Target TAGE.
    #[serde(alias = "SC-L-TAGE")]
    ScLTage,
}

/// Specifies the memory dependence prediction algorithm used to determine
/// whether loads can bypass older unresolved stores at issue time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MemDepPredictor {
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
/// use rvsim_core::config::{Config, BranchPredictor, Prefetcher};
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
#[derive(Debug, Clone, Default, Deserialize)]
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

    /// Initial stack pointer (only used when `direct_mode` is true). Defaults to `ram_base` + 16MiB if not set.
    #[serde(default)]
    pub initial_sp: Option<u64>,
}

impl GeneralConfig {
    /// Returns the default starting program counter.
    const fn default_start_pc() -> u64 {
        defaults::RAM_BASE
    }

    /// Default direct mode to true so bare-metal runs work out of the box.
    const fn default_direct_mode() -> bool {
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

    /// `VirtIO` disk MMIO base address
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

    /// Core clock in MHz; see [`defaults::CPU_CLOCK_MHZ`].
    #[serde(default = "SystemConfig::default_cpu_clock_mhz")]
    pub cpu_clock_mhz: u64,

    /// Wall-clock time the RTC reports at cycle zero, in seconds since the
    /// Unix epoch; see [`defaults::RTC_EPOCH_SECONDS`].
    #[serde(default = "SystemConfig::default_rtc_epoch_seconds")]
    pub rtc_epoch_seconds: u64,

    /// When true, UART output goes to stderr (for visibility when run from Python).
    #[serde(default)]
    pub uart_to_stderr: bool,

    /// When true, UART output is suppressed entirely (for scripting / benchmarks).
    #[serde(default)]
    pub uart_quiet: bool,

    /// Time every device takes to answer a register access, in nanoseconds
    /// (gem5's `pio_latency`).
    #[serde(default = "SystemConfig::default_device_latency_ns")]
    pub device_latency_ns: u64,

    /// Per-device access latency in nanoseconds, by device name (`UART0`,
    /// `CLINT`, `PLIC`, `VirtIO-Blk`, `SysCon`, `GoldfishRTC`, `HTIF`),
    /// overriding `device_latency_ns`.
    #[serde(default)]
    pub device_latency_ns_overrides: std::collections::HashMap<String, u64>,

    /// HTIF tohost address (0 = disabled). When non-zero, an HTIF device is
    /// registered at this address to intercept riscv-tests pass/fail writes.
    #[serde(default)]
    pub tohost_addr: u64,

    /// Number of harts in the simulated `SoC`. Default 1 (single-hart).
    /// Multi-hart support lands later in Phase C; this knob exists from
    /// Phase A so config plumbing is in place when n>1 is enabled.
    #[serde(default = "SystemConfig::default_hart_count")]
    pub hart_count: usize,
}

impl SystemConfig {
    /// Returns the default UART MMIO base address.
    const fn default_uart_base() -> u64 {
        defaults::UART_BASE
    }

    /// Returns the default `VirtIO` disk MMIO base address.
    const fn default_disk_base() -> u64 {
        defaults::DISK_BASE
    }

    /// Returns the default RAM base address.
    const fn default_ram_base() -> u64 {
        defaults::RAM_BASE
    }

    /// Returns the default CLINT MMIO base address.
    const fn default_clint_base() -> u64 {
        defaults::CLINT_BASE
    }

    /// Returns the default system controller MMIO base address.
    const fn default_syscon_base() -> u64 {
        defaults::SYSCON_BASE
    }

    /// Returns the default kernel load offset from RAM base.
    const fn default_kernel_offset() -> u64 {
        defaults::KERNEL_OFFSET
    }

    /// Returns the default system bus width in bytes.
    const fn default_bus_width() -> u64 {
        defaults::BUS_WIDTH
    }

    /// Returns the default system bus latency in cycles.
    const fn default_bus_latency() -> u64 {
        defaults::BUS_LATENCY
    }

    /// Returns the default CLINT timer divider value.
    const fn default_clint_divider() -> u64 {
        defaults::CLINT_DIVIDER
    }

    const fn default_cpu_clock_mhz() -> u64 {
        defaults::CPU_CLOCK_MHZ
    }

    const fn default_device_latency_ns() -> u64 {
        defaults::DEVICE_LATENCY_NS
    }

    /// Cycles a device takes to answer a register access unless overridden.
    #[must_use]
    pub const fn default_device_access_cycles(&self) -> u64 {
        self.ns_to_cycles(self.device_latency_ns)
    }

    /// Cycles the device named `name` takes to answer a register access.
    #[must_use]
    pub fn device_access_cycles(&self, name: &str) -> u64 {
        self.device_latency_ns_overrides
            .get(name)
            .map_or_else(|| self.default_device_access_cycles(), |&ns| self.ns_to_cycles(ns))
    }

    /// Core cycles in `ns` nanoseconds.
    const fn ns_to_cycles(&self, ns: u64) -> u64 {
        ns * self.cpu_clock_mhz / 1000
    }

    const fn default_rtc_epoch_seconds() -> u64 {
        defaults::RTC_EPOCH_SECONDS
    }

    /// Returns the default hart count (1).
    const fn default_hart_count() -> usize {
        1
    }
}

impl Default for SystemConfig {
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
            cpu_clock_mhz: defaults::CPU_CLOCK_MHZ,
            rtc_epoch_seconds: defaults::RTC_EPOCH_SECONDS,
            uart_to_stderr: false,
            uart_quiet: false,
            device_latency_ns: defaults::DEVICE_LATENCY_NS,
            device_latency_ns_overrides: std::collections::HashMap::new(),
            tohost_addr: 0,
            hart_count: 1,
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

    /// DDR5 controller parameters; used when `controller` is `Ddr5`.
    #[serde(default)]
    pub ddr5: crate::soc::memory::ddr5::Ddr5Params,

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

    /// Bandwidth of the Simple controller in GiB/s; requests are
    /// serialised on it, each busying the controller for its bytes' time.
    #[serde(default = "MemoryConfig::default_simple_bandwidth_gib_s")]
    pub simple_bandwidth_gib_s: f64,

    /// Number of DRAM banks per rank
    #[serde(default = "MemoryConfig::default_num_banks")]
    pub num_banks: usize,

    /// Row-to-Row Delay (different bank activation spacing)
    #[serde(default = "MemoryConfig::default_t_rrd")]
    pub t_rrd: u64,

    /// DRAM row (page) size in bytes
    #[serde(default = "MemoryConfig::default_row_size")]
    pub row_size_bytes: usize,

    /// Refresh interval in cycles
    #[serde(default = "MemoryConfig::default_t_refi")]
    pub t_refi: u64,

    /// Refresh cycle time in cycles
    #[serde(default = "MemoryConfig::default_t_rfc")]
    pub t_rfc: u64,

    /// L1 TLB entry count
    #[serde(default = "MemoryConfig::default_tlb_size")]
    pub tlb_size: usize,

    /// L1 TLB associativity (ways per set); 0 for fully associative
    #[serde(default = "MemoryConfig::default_tlb_ways")]
    pub tlb_ways: usize,

    /// L2 TLB entry count (shared between iTLB and dTLB); 0 for none
    #[serde(default = "MemoryConfig::default_l2_tlb_size")]
    pub l2_tlb_size: usize,

    /// L2 TLB associativity (ways per set)
    #[serde(default = "MemoryConfig::default_l2_tlb_ways")]
    pub l2_tlb_ways: usize,

    /// L2 TLB hit latency in cycles
    #[serde(default = "MemoryConfig::default_l2_tlb_latency")]
    pub l2_tlb_latency: u64,

    /// Trap on misaligned memory accesses instead of handling them natively.
    /// When true, misaligned loads/stores raise `LoadAddressMisaligned` /
    /// `StoreAddressMisaligned` exceptions (matching spike's default behavior).
    /// When false, misaligned accesses are handled transparently with a latency
    /// penalty (like many modern RISC-V cores). Default: true.
    #[serde(default = "MemoryConfig::default_misaligned_access_trap")]
    pub misaligned_access_trap: bool,

    /// Highest SATP paging mode the CPU's CSR writer will accept.
    ///
    /// Anything stronger than this is coerced to Bare on write. Lets test
    /// configurations pin the active mode without rebuilding the kernel
    /// (e.g. force a Sv57-aware Linux to fall back to Sv39). Accepted JSON
    /// values: `"bare"`, `"sv39"`, `"sv48"`, `"sv57"`. Default: Sv57 (no cap).
    #[serde(
        default = "MemoryConfig::default_paging_mode_max",
        deserialize_with = "deserialize_paging_mode"
    )]
    pub paging_mode_max: crate::core::arch::csr::PagingMode,
}

fn deserialize_misa<'de, D>(
    deserializer: D,
) -> Result<Option<crate::core::arch::csr::Misa>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|isa| isa.parse().map_err(serde::de::Error::custom))
        .transpose()
}

fn deserialize_paging_mode<'de, D>(
    deserializer: D,
) -> Result<crate::core::arch::csr::PagingMode, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use crate::core::arch::csr::PagingMode;
    use serde::de::{Error, Unexpected};

    let s = String::deserialize(deserializer)?;
    match s.to_ascii_lowercase().as_str() {
        "bare" => Ok(PagingMode::Bare),
        "sv39" => Ok(PagingMode::Sv39),
        "sv48" => Ok(PagingMode::Sv48),
        "sv57" => Ok(PagingMode::Sv57),
        _ => Err(D::Error::invalid_value(
            Unexpected::Str(&s),
            &"one of \"bare\", \"sv39\", \"sv48\", \"sv57\"",
        )),
    }
}

impl MemoryConfig {
    /// The Simple controller's bandwidth in bytes per second; `None` when
    /// it is not positive.
    #[must_use]
    pub fn simple_bandwidth_bytes_per_second(&self) -> Option<std::num::NonZeroU64> {
        let gib_s = self.simple_bandwidth_gib_s;
        if !(gib_s.is_finite() && gib_s > 0.0) {
            return None;
        }
        std::num::NonZeroU64::new((gib_s * f64::from(1u32 << 30)) as u64)
    }

    /// Returns the default RAM size in bytes.
    const fn default_ram_size() -> usize {
        defaults::RAM_SIZE
    }

    /// Returns the default CAS latency in DRAM cycles.
    const fn default_t_cas() -> u64 {
        defaults::T_CAS
    }

    /// Returns the default RAS latency in DRAM cycles.
    const fn default_t_ras() -> u64 {
        defaults::T_RAS
    }

    /// Returns the default precharge latency in DRAM cycles.
    const fn default_t_pre() -> u64 {
        defaults::T_PRE
    }

    /// Returns the default row buffer miss penalty in DRAM cycles.
    const fn default_row_miss() -> u64 {
        defaults::ROW_MISS_LATENCY
    }

    const fn default_simple_bandwidth_gib_s() -> f64 {
        defaults::SIMPLE_BANDWIDTH_GIB_S
    }

    /// Returns the default number of DRAM banks.
    const fn default_num_banks() -> usize {
        defaults::NUM_BANKS
    }

    /// Returns the default row-to-row delay in DRAM cycles.
    const fn default_t_rrd() -> u64 {
        defaults::T_RRD
    }

    /// Returns the default row size in bytes.
    const fn default_row_size() -> usize {
        defaults::ROW_SIZE_BYTES
    }

    /// Returns the default refresh interval in cycles.
    const fn default_t_refi() -> u64 {
        defaults::T_REFI
    }

    /// Returns the default refresh cycle time in cycles.
    const fn default_t_rfc() -> u64 {
        defaults::T_RFC
    }

    /// Returns the default TLB entry count.
    const fn default_tlb_size() -> usize {
        defaults::TLB_SIZE
    }

    /// Returns the default L1 TLB associativity.
    const fn default_tlb_ways() -> usize {
        defaults::TLB_WAYS
    }

    /// Returns the default L2 TLB entry count.
    const fn default_l2_tlb_size() -> usize {
        defaults::L2_TLB_SIZE
    }

    /// Returns the default L2 TLB associativity.
    const fn default_l2_tlb_ways() -> usize {
        defaults::L2_TLB_WAYS
    }

    /// Returns the default L2 TLB hit latency.
    const fn default_l2_tlb_latency() -> u64 {
        defaults::L2_TLB_LATENCY
    }

    /// Returns the default value for misaligned access trap behavior.
    ///
    /// Default `true` matches spike and avoids the cross-page corruption bug:
    /// Misaligned accesses are handled in hardware by the unaligned access unit.
    /// Keep this `false` to allow hardware misaligned handling by default.
    const fn default_misaligned_access_trap() -> bool {
        false
    }

    /// Default paging-mode cap: accept every supported mode.
    const fn default_paging_mode_max() -> crate::core::arch::csr::PagingMode {
        crate::core::arch::csr::PagingMode::Sv57
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            ram_size: defaults::RAM_SIZE,
            controller: MemoryController::default(),
            ddr5: crate::soc::memory::ddr5::Ddr5Params::default(),
            t_cas: defaults::T_CAS,
            t_ras: defaults::T_RAS,
            t_pre: defaults::T_PRE,
            row_miss_latency: defaults::ROW_MISS_LATENCY,
            simple_bandwidth_gib_s: defaults::SIMPLE_BANDWIDTH_GIB_S,
            num_banks: defaults::NUM_BANKS,
            t_rrd: defaults::T_RRD,
            row_size_bytes: defaults::ROW_SIZE_BYTES,
            t_refi: defaults::T_REFI,
            t_rfc: defaults::T_RFC,
            tlb_size: defaults::TLB_SIZE,
            tlb_ways: defaults::TLB_WAYS,
            l2_tlb_size: defaults::L2_TLB_SIZE,
            l2_tlb_ways: defaults::L2_TLB_WAYS,
            l2_tlb_latency: defaults::L2_TLB_LATENCY,
            misaligned_access_trap: false,
            paging_mode_max: crate::core::arch::csr::PagingMode::Sv57,
        }
    }
}

/// Cache hierarchy configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CacheHierarchyConfig {
    /// L1 instruction cache
    pub l1_i: CacheConfig,
    /// L1 data cache
    pub l1_d: CacheConfig,
    /// Unified L2 cache
    pub l2: CacheConfig,
    /// Unified L3 cache (optional)
    pub l3: CacheConfig,
    /// Inclusion policy for the cache hierarchy
    #[serde(default)]
    pub inclusion_policy: InclusionPolicy,
    /// Number of Write Combining Buffer entries (0 = disabled)
    #[serde(default)]
    pub wcb_entries: usize,
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

    /// Cycles from a line arriving from the next level to the requests
    /// waiting on it being answered: the fill is forwarded to them as it is
    /// written into the array (gem5's `response_latency`).
    #[serde(default = "CacheConfig::default_response_latency")]
    pub response_latency: u64,

    /// Hardware prefetcher type
    #[serde(default)]
    pub prefetcher: Prefetcher,

    /// Prefetcher table size (for stride prefetcher)
    #[serde(default = "CacheConfig::default_prefetch_table")]
    pub prefetch_table_size: usize,

    /// Prefetch degree (lines to prefetch per trigger)
    #[serde(default = "CacheConfig::default_prefetch_degree")]
    pub prefetch_degree: usize,

    /// Number of MSHRs (Miss Status Holding Registers): outstanding line
    /// fetches this level can have in flight. Zero behaves as one.
    #[serde(default = "CacheConfig::default_mshr_count")]
    pub mshr_count: usize,

    /// Writeback buffer entries: victims in flight to the next level before
    /// the cache stops accepting requests. Zero behaves as one.
    #[serde(default = "CacheConfig::default_write_buffers")]
    pub write_buffers: usize,

    /// Requests one MSHR can hold (gem5's `tgts_per_mshr`): once a line in
    /// flight has this many waiting, the cache accepts nothing until that
    /// line's fill returns. Zero behaves as one.
    #[serde(default = "CacheConfig::default_targets_per_mshr")]
    pub targets_per_mshr: usize,
}

impl CacheConfig {
    /// Returns the default cache size in bytes.
    const fn default_size() -> usize {
        defaults::CACHE_SIZE
    }

    /// Returns the default cache line size in bytes.
    const fn default_line() -> usize {
        defaults::CACHE_LINE
    }

    /// Returns the default cache associativity (number of ways).
    const fn default_ways() -> usize {
        defaults::CACHE_WAYS
    }

    /// Returns the default cache access latency in cycles.
    const fn default_latency() -> u64 {
        defaults::CACHE_LATENCY
    }

    /// Returns the default fill-forwarding latency.
    const fn default_response_latency() -> u64 {
        defaults::CACHE_RESPONSE_LATENCY
    }

    /// Returns the default prefetcher pattern table size.
    const fn default_prefetch_table() -> usize {
        defaults::PREFETCH_TABLE_SIZE
    }

    /// Returns the default prefetch degree (lines per trigger).
    const fn default_prefetch_degree() -> usize {
        defaults::PREFETCH_DEGREE
    }

    /// Returns the default MSHR count.
    const fn default_mshr_count() -> usize {
        defaults::MSHR_COUNT
    }

    /// Returns the default writeback buffer size.
    const fn default_write_buffers() -> usize {
        defaults::WRITE_BUFFERS
    }

    /// Returns the default number of requests one MSHR can hold.
    const fn default_targets_per_mshr() -> usize {
        defaults::TARGETS_PER_MSHR
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            size_bytes: defaults::CACHE_SIZE,
            line_bytes: defaults::CACHE_LINE,
            ways: defaults::CACHE_WAYS,
            policy: ReplacementPolicy::default(),
            latency: defaults::CACHE_LATENCY,
            response_latency: defaults::CACHE_RESPONSE_LATENCY,
            prefetcher: Prefetcher::default(),
            prefetch_table_size: defaults::PREFETCH_TABLE_SIZE,
            prefetch_degree: defaults::PREFETCH_DEGREE,
            mshr_count: defaults::MSHR_COUNT,
            write_buffers: defaults::WRITE_BUFFERS,
            targets_per_mshr: defaults::TARGETS_PER_MSHR,
        }
    }
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
    pub branch_predictor: BranchPredictor,

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
    pub misa_override: Option<crate::core::arch::csr::Misa>,

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
    pub mem_dep_predictor: MemDepPredictor,

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
    pub vec_store_forwarding: crate::core::pipeline::vec_store_buffer::VecStoreForwarding,
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
            branch_predictor: BranchPredictor::default(),
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
            mem_dep_predictor: MemDepPredictor::default(),
            store_set: StoreSetConfig::default(),
            vlen: 128,
            num_vec_lanes: None,
            vector_mem_width: None,
            prf_vpr_size: 64,
            vec_chaining: true,
            vec_store_buffer_size: defaults::VEC_STORE_BUFFER_SIZE,
            vec_store_forwarding:
                crate::core::pipeline::vec_store_buffer::VecStoreForwarding::ByteMask,
        }
    }
}

/// TAGE (Tagged Geometric) predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TageConfig {
    /// Number of tagged tables
    #[serde(default = "TageConfig::default_banks")]
    pub num_banks: usize,

    /// Entries per table
    #[serde(default = "TageConfig::default_table_size")]
    pub table_size: usize,

    /// Useful counter reset interval
    #[serde(default = "TageConfig::default_reset_interval")]
    pub reset_interval: u32,

    /// History lengths for each bank
    #[serde(default = "TageConfig::default_history_lengths")]
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    #[serde(default = "TageConfig::default_tag_widths")]
    pub tag_widths: Vec<usize>,

    /// `USE_ALT_ON_NA` counters. One is `TAGEBase`'s; more are indexed by
    /// the provider's bank group and the alternate's confidence, as
    /// TAGE-SC-L indexes its 16.
    #[serde(default = "TageConfig::default_use_alt_counters")]
    pub use_alt_counters: usize,

    /// Width of each `USE_ALT_ON_NA` counter.
    #[serde(default = "TageConfig::default_use_alt_bits")]
    pub use_alt_bits: u32,

    /// Width of each tagged entry's useful counter.
    #[serde(default = "TageConfig::default_useful_bits")]
    pub useful_bits: u32,

    /// Most entries one misprediction allocates.
    #[serde(default = "TageConfig::default_max_allocations")]
    pub max_allocations: usize,

    /// How a misprediction takes new entries and how useful bits age.
    #[serde(default)]
    pub allocation: TageAllocation,

    /// Which entries a committed branch trains.
    #[serde(default)]
    pub update: TageUpdate,
}

/// How TAGE allocates entries for a misprediction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TageAllocation {
    /// `TAGEBase`: free entries from one of the next three tables up,
    /// forcing one free when none is; useful bits halve every
    /// `reset_interval` updates.
    #[default]
    TageBase,
    /// CBP-5 TAGE-SC-L: pairs of tables from a randomised start, decaying
    /// strong unuseful entries it passes; useful bits halve once the
    /// allocations that found no free entry outweigh those that did by
    /// `reset_interval`. A branch the final prediction got right allocates
    /// one time in 32.
    Cbp5,
}

/// Which entries TAGE trains on a committed branch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TageUpdate {
    /// `TAGEBase`: the provider, the alternate while the provider is not
    /// useful, and the provider's useful bit when it and the alternate
    /// disagree.
    #[default]
    TageBase,
    /// CBP-5 TAGE-SC-L: the alternate only when a weak provider is wrong;
    /// a provider turning weak, or right beside a saturated right
    /// alternate, loses its useful bit.
    Cbp5,
}

impl Default for TageConfig {
    fn default() -> Self {
        Self {
            num_banks: Self::default_banks(),
            table_size: Self::default_table_size(),
            reset_interval: Self::default_reset_interval(),
            history_lengths: Self::default_history_lengths(),
            tag_widths: Self::default_tag_widths(),
            use_alt_counters: Self::default_use_alt_counters(),
            use_alt_bits: Self::default_use_alt_bits(),
            useful_bits: Self::default_useful_bits(),
            max_allocations: Self::default_max_allocations(),
            allocation: TageAllocation::default(),
            update: TageUpdate::default(),
        }
    }
}

impl TageConfig {
    /// Returns the default number of TAGE predictor banks.
    const fn default_banks() -> usize {
        defaults::TAGE_BANKS
    }

    /// Returns the default TAGE predictor table size per bank.
    const fn default_table_size() -> usize {
        defaults::TAGE_TABLE_SIZE
    }

    /// Returns the default TAGE useful counter reset interval.
    const fn default_reset_interval() -> u32 {
        defaults::TAGE_RESET_INTERVAL
    }

    /// Returns the default history lengths for each TAGE bank.
    ///
    /// Geometric progression: [5, 11, 22, 44, 89, 178, 356, 712] (~2× ratio).
    fn default_history_lengths() -> Vec<usize> {
        vec![5, 11, 22, 44, 89, 178, 356, 712]
    }

    /// Returns the default tag widths for each TAGE bank.
    ///
    /// Tag widths increase with history length: [8, 8, 9, 9, 10, 10, 11, 11] bits.
    fn default_tag_widths() -> Vec<usize> {
        vec![8, 8, 9, 9, 10, 10, 11, 11]
    }

    const fn default_use_alt_counters() -> usize {
        1
    }

    const fn default_use_alt_bits() -> u32 {
        4
    }

    const fn default_useful_bits() -> u32 {
        2
    }

    const fn default_max_allocations() -> usize {
        1
    }
}

/// One GEHL component of the statistical corrector.
///
/// A table of counters per history length, each indexed by the PC hashed
/// with that many bits of the component's history, and a weight on the
/// component's sum.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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

/// Seznec's loop predictor (used by SC-L-TAGE). The defaults are gem5's
/// 64KB TAGE-SC-L loop predictor.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct LoopConfig {
    /// The table holds `2^log_size` entries.
    pub log_size: usize,
    /// In sets of `2^log_assoc` ways.
    pub log_assoc: usize,
    /// Tag bits per entry.
    pub tag_bits: usize,
    /// Iteration-count bits per entry.
    pub iter_bits: usize,
    /// Confidence bits per entry; a saturated counter predicts.
    pub confidence_bits: usize,
    /// Age bits per entry, which replacement consumes.
    pub age_bits: usize,
    /// Bits of the `WITHLOOP` counter that decides whether loop
    /// predictions are used.
    pub use_counter_bits: usize,
    /// Each entry learns whether its loop body is taken or not taken.
    pub use_direction_bit: bool,
    /// The set and tag hash the PC rather than slice it.
    pub use_hashing: bool,
    /// Allocate on one mispredict in four, trying one way.
    pub restrict_allocation: bool,
    /// Iteration count a new entry starts with.
    pub initial_iter: u16,
    /// Age a new entry starts with.
    pub initial_age: u8,
    /// Freeing an entry's count also clears its age.
    pub optional_age_reset: bool,
    /// A long loop predicts before its confidence saturates, once
    /// confidence × iterations exceeds 128 (TAGE-SC-L's rule).
    pub long_loop_confidence: bool,
    /// A correct loop prediction ages its entry up one time in eight even
    /// when TAGE was also right (TAGE-SC-L's rule).
    pub optional_age_increment: bool,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            log_size: 5,
            log_assoc: 2,
            tag_bits: 10,
            iter_bits: 10,
            confidence_bits: 4,
            age_bits: 4,
            use_counter_bits: 7,
            use_direction_bit: true,
            use_hashing: true,
            restrict_allocation: true,
            initial_iter: 0,
            initial_age: 7,
            optional_age_reset: false,
            long_loop_confidence: true,
            optional_age_increment: true,
        }
    }
}

/// Indirect Target TAGE configuration (used by SC-L-TAGE).
#[derive(Debug, Clone, Deserialize)]
pub struct IttageConfig {
    /// Number of tagged tables
    #[serde(default = "IttageConfig::default_num_banks")]
    pub num_banks: usize,

    /// Entries per table
    #[serde(default = "IttageConfig::default_table_size")]
    pub table_size: usize,

    /// History lengths for each bank
    #[serde(default = "IttageConfig::default_history_lengths")]
    pub history_lengths: Vec<usize>,

    /// Tag widths for each bank
    #[serde(default = "IttageConfig::default_tag_widths")]
    pub tag_widths: Vec<usize>,

    /// Useful counter reset interval
    #[serde(default = "IttageConfig::default_reset_interval")]
    pub reset_interval: u32,
}

impl Default for IttageConfig {
    fn default() -> Self {
        Self {
            num_banks: Self::default_num_banks(),
            table_size: Self::default_table_size(),
            history_lengths: Self::default_history_lengths(),
            tag_widths: Self::default_tag_widths(),
            reset_interval: Self::default_reset_interval(),
        }
    }
}

impl IttageConfig {
    const fn default_num_banks() -> usize {
        8
    }

    const fn default_table_size() -> usize {
        256
    }

    fn default_history_lengths() -> Vec<usize> {
        vec![4, 8, 16, 32, 64, 128, 256, 512]
    }

    fn default_tag_widths() -> Vec<usize> {
        vec![9, 9, 10, 10, 11, 11, 12, 12]
    }

    const fn default_reset_interval() -> u32 {
        256_000
    }
}

/// Perceptron branch predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PerceptronConfig {
    /// Global history length
    #[serde(default = "PerceptronConfig::default_history")]
    pub history_length: usize,

    /// Log2 of perceptron table size
    #[serde(default = "PerceptronConfig::default_table_bits")]
    pub table_bits: usize,
}

impl Default for PerceptronConfig {
    fn default() -> Self {
        Self { history_length: Self::default_history(), table_bits: Self::default_table_bits() }
    }
}

impl PerceptronConfig {
    /// Returns the default Perceptron predictor global history length.
    const fn default_history() -> usize {
        defaults::PERCEPTRON_HISTORY
    }

    /// Returns the default Perceptron predictor table size (log2).
    const fn default_table_bits() -> usize {
        defaults::PERCEPTRON_TABLE_BITS
    }
}

/// Tournament branch predictor configuration.
#[derive(Debug, Clone, Deserialize)]
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

impl Default for TournamentConfig {
    fn default() -> Self {
        Self {
            global_size_bits: Self::default_global(),
            local_hist_bits: Self::default_local_hist(),
            local_pred_bits: Self::default_local_pred(),
        }
    }
}

impl TournamentConfig {
    /// Returns the default Tournament predictor global history table size (log2).
    const fn default_global() -> usize {
        defaults::TOURNAMENT_GLOBAL_BITS
    }

    /// Returns the default Tournament predictor local history table size (log2).
    const fn default_local_hist() -> usize {
        defaults::TOURNAMENT_LOCAL_HIST_BITS
    }

    /// Returns the default Tournament predictor local prediction table size (log2).
    const fn default_local_pred() -> usize {
        defaults::TOURNAMENT_LOCAL_PRED_BITS
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

/// Which home agent decides who must be snooped.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "kind")]
pub enum HomeAgentConfig {
    /// Snoop every other core on every request.
    Broadcast,
    /// Exact sharer tracking in a set-associative filter.
    SnoopFilter {
        /// Tracked lines as a multiple of the aggregate private L2 lines.
        #[serde(default = "HomeAgentConfig::default_capacity_factor")]
        capacity_factor: f64,
        /// Filter associativity.
        #[serde(default = "HomeAgentConfig::default_ways")]
        ways: usize,
    },
}

impl HomeAgentConfig {
    const fn default_capacity_factor() -> f64 {
        1.5
    }

    const fn default_ways() -> usize {
        8
    }
}

impl Default for HomeAgentConfig {
    fn default() -> Self {
        Self::SnoopFilter {
            capacity_factor: Self::default_capacity_factor(),
            ways: Self::default_ways(),
        }
    }
}

/// Which interconnect carries coherence messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind")]
pub enum InterconnectConfig {
    /// Any port to any port, one hop.
    Crossbar {
        /// Cycles a message spends crossing.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes an output port moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// Bidirectional ring; the home is one stop.
    Ring {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// 2-D mesh with XY routing.
    Mesh {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// 2-D torus (mesh with wraparound) with XY routing.
    Torus {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// Hypercube with dimension-order routing.
    Hypercube {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
}

impl InterconnectConfig {
    const fn default_hop_latency() -> u64 {
        2
    }

    const fn default_bytes_per_cycle() -> usize {
        32
    }
}

impl Default for InterconnectConfig {
    fn default() -> Self {
        Self::Crossbar {
            hop_latency: Self::default_hop_latency(),
            bytes_per_cycle: Self::default_bytes_per_cycle(),
        }
    }
}

/// Coherence protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CoherenceProtocolConfig {
    /// Modified / Exclusive / Shared / Invalid.
    #[default]
    Mesi,
}

/// Coherence fabric configuration.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CoherenceConfig {
    /// Protocol.
    #[serde(default)]
    pub protocol: CoherenceProtocolConfig,
    /// Home agent.
    #[serde(default)]
    pub home_agent: HomeAgentConfig,
    /// Interconnect.
    #[serde(default)]
    pub interconnect: InterconnectConfig,
    /// Transactions the home can have live at once.
    #[serde(default = "CoherenceConfig::default_txn_entries")]
    pub txn_entries: usize,
}

impl CoherenceConfig {
    const fn default_txn_entries() -> usize {
        32
    }
}

impl Default for CoherenceConfig {
    fn default() -> Self {
        Self {
            protocol: CoherenceProtocolConfig::default(),
            home_agent: HomeAgentConfig::default(),
            interconnect: InterconnectConfig::default(),
            txn_entries: Self::default_txn_entries(),
        }
    }
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
    /// `misa_override` sets V, which needs VLEN >= 128 and ELEN = 64.
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
    /// RV64IMAFDC plus V when the vector unit is the full V extension.
    #[must_use]
    pub fn misa(&self) -> crate::core::arch::csr::Misa {
        self.pipeline
            .misa_override
            .unwrap_or_else(|| crate::core::arch::csr::Misa::rv64imafdc(self.implements_full_v()))
    }

    /// True when the vector unit meets V's minimum: VLEN >= 128 (Zvl128b)
    /// and ELEN = 64 (Zve64d).
    const fn implements_full_v(&self) -> bool {
        self.pipeline.vlen >= 128 && self.isa.vector.elen == 64
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
                vlen: self.pipeline.vlen,
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

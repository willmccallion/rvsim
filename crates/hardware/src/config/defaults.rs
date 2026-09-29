//! Default configuration constants for the simulator.
//!
//! These values define the baseline hardware configuration when not
//! explicitly overridden in TOML configuration files.

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

/// Base address of virtio block device MMIO region.
pub const DISK_BASE: u64 = 0x9000_0000;

/// Base address of CLINT (Core Local Interruptor) timer MMIO region.
pub const CLINT_BASE: u64 = 0x0200_0000;

/// Base address of system controller (power/reset) MMIO region.
pub const SYSCON_BASE: u64 = 0x0010_0000;

/// Base address of the simulator control MMIO region.
pub const SIM_CONTROL_BASE: u64 = 0x0010_2000;

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

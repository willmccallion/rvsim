//! Main memory: controller, size and the translation options that depend on it.

use super::defaults;
use serde::{Deserialize, Serialize};

/// Memory controller implementation types.
///
/// Specifies the type of memory controller used to model main memory
/// access timing and behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum MemoryControllerKind {
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
    Dram,
    /// DDR5 command-level controller (JEDEC-timed).
    ///
    /// Per-bank command state machines across channels, sub-channels and
    /// ranks, JEDEC timing from a speed bin, FR-FCFS scheduling, refresh,
    /// power-down and ECC scrubbing. Parameters come from
    /// [`MemoryConfig::ddr5`].
    Ddr5,
}

/// Main memory system configuration.
///
/// Specifies RAM size, memory controller type, DRAM timing parameters,
/// and TLB configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct MemoryConfig {
    /// RAM size in bytes
    pub ram_size: usize,

    /// Memory controller type
    pub controller: MemoryControllerKind,

    /// DDR5 controller parameters; used when `controller` is `Ddr5`.
    pub ddr5: crate::config::ddr5::Ddr5Params,

    /// CAS latency (column access strobe)
    pub t_cas: u64,

    /// RAS latency (row access strobe)
    pub t_ras: u64,

    /// Precharge latency
    pub t_pre: u64,

    /// Latency of the Simple controller in core cycles.
    pub simple_latency: u64,

    /// Bandwidth of the Simple controller in GiB/s; requests are
    /// serialised on it, each busying the controller for its bytes' time.
    pub simple_bandwidth_gib_s: f64,

    /// Number of DRAM banks per rank
    pub num_banks: usize,

    /// Row-to-Row Delay (different bank activation spacing)
    pub t_rrd: u64,

    /// DRAM row (page) size in bytes
    pub row_size_bytes: usize,

    /// Refresh interval in cycles
    pub t_refi: u64,

    /// Refresh cycle time in cycles
    pub t_rfc: u64,

    /// L1 TLB entry count
    pub tlb_size: usize,

    /// L1 TLB associativity (ways per set); 0 for fully associative
    pub tlb_ways: usize,

    /// L2 TLB entry count (shared between iTLB and dTLB); 0 for none
    pub l2_tlb_size: usize,

    /// L2 TLB associativity (ways per set)
    pub l2_tlb_ways: usize,

    /// L2 TLB hit latency in cycles
    pub l2_tlb_latency: u64,

    /// Trap on misaligned memory accesses instead of handling them natively.
    /// When true, misaligned loads/stores raise `LoadAddressMisaligned` /
    /// `StoreAddressMisaligned` exceptions (matching spike's default behavior).
    /// When false, misaligned accesses are handled transparently with a latency
    /// penalty (like many modern RISC-V cores). Default: true.
    pub misaligned_access_trap: bool,

    /// Highest SATP paging mode the CPU's CSR writer will accept.
    ///
    /// Anything stronger than this is coerced to Bare on write. Lets test
    /// configurations pin the active mode without rebuilding the kernel
    /// (e.g. force a Sv57-aware Linux to fall back to Sv39): `"Bare"`,
    /// `"Sv39"`, `"Sv48"` or `"Sv57"`, the default (no cap).
    pub paging_mode_max: crate::isa::privileged::PagingMode,
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
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            ram_size: defaults::RAM_SIZE,
            controller: MemoryControllerKind::default(),
            ddr5: crate::config::ddr5::Ddr5Params::default(),
            t_cas: defaults::T_CAS,
            t_ras: defaults::T_RAS,
            t_pre: defaults::T_PRE,
            simple_latency: defaults::SIMPLE_LATENCY,
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
            paging_mode_max: crate::isa::privileged::PagingMode::Sv57,
        }
    }
}

/// Address-bit interleave strategy. Names read high-order to low-order bit,
/// so `RoRaBaChCo` uses `column` as the lowest-order bits (best for burst
/// spatial locality on a single channel).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, serde::Deserialize, serde::Serialize)]
pub enum AddressMappingKind {
    /// Row : Rank : Bank(Group+Bank) : Channel : Column.
    /// gem5 default; good spatial locality for sequential streams.
    #[default]
    RoRaBaChCo,
    /// Row : Rank : Bank(Group+Bank) : Column : Channel.
    /// Channel-interleaved at cache-line granularity; higher aggregate BW
    /// under strided workloads, lower row-buffer reuse.
    RoRaBaCoCh,
    /// Row : Column : Rank : Bank(Group+Bank) : Channel.
    /// Open-page-friendly for small working sets.
    RoCoRaBaCh,
}

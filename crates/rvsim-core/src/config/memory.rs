//! Main memory: controller, size and the translation options that depend on it.

use super::defaults;
use serde::Deserialize;

/// Memory controller implementation types.
///
/// Specifies the type of memory controller used to model main memory
/// access timing and behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
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

/// Main memory system configuration.
///
/// Specifies RAM size, memory controller type, DRAM timing parameters,
/// and TLB configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    /// RAM size in bytes
    #[serde(default = "MemoryConfig::default_ram_size")]
    pub ram_size: usize,

    /// Memory controller type
    #[serde(default)]
    pub controller: MemoryControllerKind,

    /// DDR5 controller parameters; used when `controller` is `Ddr5`.
    #[serde(default)]
    pub ddr5: crate::config::ddr5::Ddr5Params,

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
    pub paging_mode_max: crate::isa::privileged::PagingMode,
}

fn deserialize_paging_mode<'de, D>(
    deserializer: D,
) -> Result<crate::isa::privileged::PagingMode, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use crate::isa::privileged::PagingMode;
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
    const fn default_paging_mode_max() -> crate::isa::privileged::PagingMode {
        crate::isa::privileged::PagingMode::Sv57
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
            paging_mode_max: crate::isa::privileged::PagingMode::Sv57,
        }
    }
}

/// Address-bit interleave strategy. Names read high-order to low-order bit,
/// so `RoRaBaChCo` uses `column` as the lowest-order bits (best for burst
/// spatial locality on a single channel).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, serde::Deserialize)]
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

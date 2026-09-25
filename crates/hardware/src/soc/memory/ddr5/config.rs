//! Configuration for [`crate::soc::memory::ddr5::controller::Ddr5Controller`].

use crate::soc::memory::address::AddressMappingKind;
use crate::soc::memory::ddr5::ecc::EccKind;
use crate::soc::memory::ddr5::refresh::RefreshKind;
use crate::soc::memory::ddr5::scheduler::SchedulerKind;
use crate::soc::memory::ddr5::timing::{Constraint, Ddr5SpeedBin, Ddr5Timing};

/// Static topology and policy parameters for a DDR5 memory subsystem.
///
/// Every count except the row/column bit widths must be a power of two so that
/// the address mapper can extract fields with pure bit-slice arithmetic. All
/// latencies are in DRAM command clocks; the controller converts to and from
/// simulator cycles using the core clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr5Config {
    /// Number of independent DRAM channels.
    pub channels: u8,
    /// Sub-channels per channel (DDR5 defines two 32-bit sub-channels).
    pub subchannels_per_channel: u8,
    /// Ranks per channel (each sub-channel sees the same rank population).
    pub ranks_per_channel: u8,
    /// Bank groups per rank.
    pub bank_groups_per_rank: u8,
    /// Banks per bank group.
    pub banks_per_group: u8,
    /// Number of row-address bits.
    pub row_bits: u8,
    /// Number of column-address bits.
    pub column_bits: u8,
    /// Row size in bytes (implied by column bits + intra-row column stride,
    /// carried explicitly for stats and validation).
    pub row_size_bytes: u32,
    /// Read queue capacity per subchannel. Reads beyond it wait for
    /// admission, as a requester waiting on a retry would.
    pub read_queue_entries: usize,
    /// Write queue capacity per subchannel.
    pub write_queue_entries: usize,
    /// Depth of the write queue at which the scheduler starts draining writes.
    pub write_high_watermark: usize,
    /// Depth of the write queue at which the scheduler returns to reads.
    pub write_low_watermark: usize,
    /// Once draining, the scheduler issues at least this many writes before
    /// switching back to reads (gem5 `min_writes_per_switch`).
    pub min_writes_per_switch: usize,
    /// Fixed controller pipeline latency charged to every access in DRAM
    /// clocks: request decode and queue insertion (gem5 `static_frontend_latency`).
    pub frontend_latency: u64,
    /// Fixed response-path latency in DRAM clocks (gem5 `static_backend_latency`).
    pub backend_latency: u64,
    /// Request selection policy.
    pub scheduler: SchedulerKind,
    /// Refresh cadence policy.
    pub refresh: RefreshKind,
    /// Rank power-down policy.
    pub power_down: PowerDownPolicy,
    /// ECC and patrol-scrub policy.
    pub ecc: EccKind,
    /// Address-bit interleave strategy.
    pub address_mapping: AddressMappingKind,
    /// Command-timing constants.
    pub timing: Ddr5Timing,
}

impl Ddr5Config {
    /// DDR5-4800B desktop configuration: two channels of two sub-channels,
    /// two ranks per channel, eight bank groups × four banks, 15-bit rows,
    /// 10-bit columns; gem5-sized queues and controller latencies.
    #[must_use]
    pub const fn ddr5_4800() -> Self {
        let bin = Ddr5SpeedBin::DDR5_4800B;
        Self {
            channels: 2,
            subchannels_per_channel: 2,
            ranks_per_channel: 2,
            bank_groups_per_rank: 8,
            banks_per_group: 4,
            row_bits: 15,
            column_bits: 10,
            row_size_bytes: 8192,
            read_queue_entries: 64,
            write_queue_entries: 64,
            write_high_watermark: 54,
            write_low_watermark: 32,
            min_writes_per_switch: 16,
            frontend_latency: Constraint::ps(10_000).cycles(bin.data_rate_mts),
            backend_latency: Constraint::ps(10_000).cycles(bin.data_rate_mts),
            scheduler: SchedulerKind::FrFcfs,
            refresh: RefreshKind::AllBank,
            power_down: PowerDownPolicy::Disabled,
            ecc: EccKind::None,
            address_mapping: AddressMappingKind::RoRaBaChCo,
            timing: Ddr5Timing::from_bin(&bin),
        }
    }
}

/// When a rank with nothing to do enters power-down.
///
/// A powered-down rank pays tXP after the exit command before its next
/// command. gem5 ships with power-down disabled (`enable_dram_powerdown`),
/// so that is the default here too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PowerDownPolicy {
    /// Ranks never power down.
    #[default]
    Disabled,
    /// A rank enters power-down once it has had no command, no burst on
    /// the data bus, and no queued request for `idle_clocks` DRAM clocks.
    AfterIdle {
        /// Idle clocks before entry.
        idle_clocks: u64,
    },
}

impl Default for Ddr5Config {
    fn default() -> Self {
        Self::ddr5_4800()
    }
}

//! The command-level DRAM controller's configuration, whatever the JEDEC
//! standard: each standard's front end (such as `ddr5`) resolves its
//! parameters into a [`DramStandardConfig`].

pub mod timing;

use std::num::NonZeroU8;

use crate::config::AddressMappingKind;
use timing::DramTiming;

/// Number of low-order address bits that are the intra-cache-line byte offset.
/// 64-byte lines → 6 bits.
pub const CACHE_LINE_OFFSET_BITS: u8 = 6;

/// Static topology and policy parameters for a DRAM memory subsystem.
///
/// Every count except the row/column bit widths must be a power of two so that
/// the address mapper can extract fields with pure bit-slice arithmetic. All
/// latencies are in DRAM command clocks; the controller converts to and from
/// simulator cycles using the core clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DramStandardConfig {
    /// Number of independent DRAM channels.
    pub channels: u8,
    /// Sub-channels per channel, each with its own command and data bus
    /// (DDR5 defines two 32-bit sub-channels).
    pub subchannels_per_channel: u8,
    /// Ranks per channel (each sub-channel sees the same rank population).
    pub ranks_per_channel: u8,
    /// Bank groups per rank.
    pub bank_groups_per_rank: u8,
    /// Banks per bank group.
    pub banks_per_group: NonZeroU8,
    /// Number of row-address bits.
    pub row_bits: u8,
    /// Number of column-address bits.
    pub column_bits: u8,
    /// Row size in bytes per sub-channel: `64 << column_bits`.
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
    pub timing: DramTiming,
    /// Command-bus clocks each command occupies.
    pub commands: CommandCycles,
}

/// Command-bus clocks each command occupies, which the standard fixes: the
/// next command on the bus waits for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandCycles {
    /// An ACTIVATE.
    pub activate: u64,
    /// A READ or WRITE.
    pub column: u64,
    /// A PRECHARGE.
    pub precharge: u64,
    /// A REFRESH.
    pub refresh: u64,
    /// A power-down entry or exit.
    pub power_down: u64,
}

/// Why a DRAM standard's parameter block was rejected; `standard` names the
/// block in the configuration (`ddr5`, `lpddr4`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DramParamsError {
    /// A topology count must be a power of two for the address mapper.
    #[error("{standard}.{field} must be a power of two, got {value}")]
    NotPowerOfTwo {
        /// The parameter block.
        standard: &'static str,
        /// Name of the offending field.
        field: &'static str,
        /// Value supplied.
        value: u8,
    },
    /// A count was zero.
    #[error("{standard}.{field} must be non-zero")]
    Zero {
        /// The parameter block.
        standard: &'static str,
        /// Name of the offending field.
        field: &'static str,
    },
    /// The write watermarks or queue sizes are inconsistent.
    #[error("{standard} write watermarks must satisfy low < high <= write_queue_entries")]
    WriteQueue {
        /// The parameter block.
        standard: &'static str,
    },
    /// A rank has more than 64 banks, which the refresh masks cannot cover.
    #[error("{standard} supports at most 64 banks per rank")]
    TooManyBanks {
        /// The parameter block.
        standard: &'static str,
    },
}

/// Checks that each named topology count is a non-zero power of two.
///
/// # Errors
///
/// The first count that is zero or not a power of two.
pub fn check_topology(
    standard: &'static str,
    counts: &[(&'static str, u8)],
) -> Result<(), DramParamsError> {
    for &(field, value) in counts {
        if value == 0 {
            return Err(DramParamsError::Zero { standard, field });
        }
        if !value.is_power_of_two() {
            return Err(DramParamsError::NotPowerOfTwo { standard, field, value });
        }
    }
    Ok(())
}

/// A parameter block's request queues and write-drain watermarks.
#[derive(Clone, Copy, Debug)]
pub struct DrainQueues {
    /// Read queue capacity.
    pub read_entries: usize,
    /// Write queue capacity.
    pub write_entries: usize,
    /// Write-queue depth that starts a drain.
    pub high_watermark: usize,
    /// Write-queue depth that ends a drain.
    pub low_watermark: usize,
}

impl DrainQueues {
    /// Checks that both queues hold something and the watermarks satisfy
    /// `low < high <= write_entries`.
    ///
    /// # Errors
    ///
    /// A queue of zero entries, or watermarks out of order.
    pub const fn check(self, standard: &'static str) -> Result<(), DramParamsError> {
        if self.write_entries == 0
            || self.high_watermark > self.write_entries
            || self.low_watermark >= self.high_watermark
        {
            return Err(DramParamsError::WriteQueue { standard });
        }
        if self.read_entries == 0 {
            return Err(DramParamsError::Zero { standard, field: "read_queue_entries" });
        }
        Ok(())
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

/// Which ECC policy a controller is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EccKind {
    /// No ECC: every line is trusted as read.
    #[default]
    None,
    /// Single-error-correct, double-error-detect side-band ECC.
    SecDed {
        /// Nanoseconds between patrol-scrub reads; `None` disables scrubbing.
        patrol_scrub_ns: Option<u64>,
    },
    /// Chipkill (symbol-correcting) side-band ECC.
    ChipKill {
        /// Nanoseconds between patrol-scrub reads; `None` disables scrubbing.
        patrol_scrub_ns: Option<u64>,
    },
}

/// Which refresh policy a controller is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub enum RefreshKind {
    /// `REFab`: every bank of a rank refreshes at once.
    #[default]
    AllBank,
    /// `REFsb`: one bank per bank group refreshes while the rest stay open.
    SameBank,
}

/// Which request scheduler a controller is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub enum SchedulerKind {
    /// First come, first served.
    Fcfs,
    /// First ready, first come, first served: row-buffer hits go first.
    #[default]
    FrFcfs,
}

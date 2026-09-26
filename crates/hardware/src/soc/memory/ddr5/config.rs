//! Configuration for [`crate::soc::memory::ddr5::controller::Ddr5Controller`].

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::soc::memory::address::AddressMappingKind;
use crate::soc::memory::ddr5::ecc::EccKind;
use crate::soc::memory::ddr5::refresh::RefreshKind;
use crate::soc::memory::ddr5::scheduler::SchedulerKind;
use crate::soc::memory::ddr5::timing::{Constraint, Ddr5SpeedBin, Ddr5Timing, Ddr5TimingField};

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
    pub timing: Ddr5Timing,
}

impl Ddr5Config {
    /// DDR5-4800B desktop configuration: two channels of two sub-channels,
    /// two ranks per channel of 16 Gb x8 devices (eight bank groups × four
    /// banks, 65536 rows, 4 KiB rows per sub-channel), gem5-sized queues
    /// and controller latencies.
    #[must_use]
    pub const fn ddr5_4800() -> Self {
        let bin = Ddr5SpeedBin::DDR5_4800B;
        Self {
            channels: 2,
            subchannels_per_channel: 2,
            ranks_per_channel: 2,
            bank_groups_per_rank: 8,
            banks_per_group: 4,
            row_bits: 16,
            column_bits: 6,
            row_size_bytes: 4096,
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

/// A JEDEC speed bin selectable from configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
pub enum Ddr5SpeedBinName {
    /// DDR5-4800B (40-39-39).
    #[default]
    #[serde(alias = "4800B", alias = "ddr5_4800b", alias = "DDR5-4800B")]
    Ddr5_4800B,
    /// DDR5-5600B (46-45-45).
    #[serde(alias = "5600B", alias = "ddr5_5600b", alias = "DDR5-5600B")]
    Ddr5_5600B,
}

impl Ddr5SpeedBinName {
    const fn bin(self) -> Ddr5SpeedBin {
        match self {
            Self::Ddr5_4800B => Ddr5SpeedBin::DDR5_4800B,
            Self::Ddr5_5600B => Ddr5SpeedBin::DDR5_5600B,
        }
    }
}

/// ECC mode selectable from configuration; scrub cadence is separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
pub enum EccMode {
    /// No ECC.
    #[default]
    None,
    /// SEC-DED side-band ECC.
    SecDed,
    /// Chipkill side-band ECC.
    ChipKill,
}

/// DDR5 parameters as they appear in the configuration file. Validated on
/// deserialization (see [`Ddr5ParamsError`]) and resolved into a
/// [`Ddr5Config`] by [`Ddr5Params::to_config`].
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(try_from = "Ddr5ParamsRaw")]
pub struct Ddr5Params {
    raw: Ddr5ParamsRaw,
}

/// Why a DDR5 parameter block was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ddr5ParamsError {
    /// A topology count must be a power of two for the address mapper.
    NotPowerOfTwo {
        /// Name of the offending field.
        field: &'static str,
        /// Value supplied.
        value: u8,
    },
    /// A topology count was zero.
    Zero {
        /// Name of the offending field.
        field: &'static str,
    },
    /// The write watermarks or queue sizes are inconsistent.
    WriteQueue,
    /// A rank has more than 64 banks, which the refresh masks cannot cover.
    TooManyBanks,
}

impl std::fmt::Display for Ddr5ParamsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPowerOfTwo { field, value } => {
                write!(f, "ddr5.{field} must be a power of two, got {value}")
            }
            Self::Zero { field } => write!(f, "ddr5.{field} must be non-zero"),
            Self::WriteQueue => {
                write!(f, "ddr5 write watermarks must satisfy low < high <= write_queue_entries")
            }
            Self::TooManyBanks => write!(f, "ddr5 supports at most 64 banks per rank"),
        }
    }
}

impl std::error::Error for Ddr5ParamsError {}

impl TryFrom<Ddr5ParamsRaw> for Ddr5Params {
    type Error = Ddr5ParamsError;

    fn try_from(raw: Ddr5ParamsRaw) -> Result<Self, Self::Error> {
        for (field, value) in [
            ("channels", raw.channels),
            ("subchannels_per_channel", raw.subchannels_per_channel),
            ("ranks_per_channel", raw.ranks_per_channel),
            ("bank_groups_per_rank", raw.bank_groups_per_rank),
            ("banks_per_group", raw.banks_per_group),
        ] {
            if value == 0 {
                return Err(Ddr5ParamsError::Zero { field });
            }
            if !value.is_power_of_two() {
                return Err(Ddr5ParamsError::NotPowerOfTwo { field, value });
            }
        }
        if u32::from(raw.bank_groups_per_rank) * u32::from(raw.banks_per_group) > 64 {
            return Err(Ddr5ParamsError::TooManyBanks);
        }
        if raw.write_queue_entries == 0
            || raw.write_high_watermark > raw.write_queue_entries
            || raw.write_low_watermark >= raw.write_high_watermark
        {
            return Err(Ddr5ParamsError::WriteQueue);
        }
        if raw.read_queue_entries == 0 {
            return Err(Ddr5ParamsError::Zero { field: "read_queue_entries" });
        }
        Ok(Self { raw })
    }
}

impl Ddr5Params {
    /// Resolves the parameters into a controller configuration.
    #[must_use]
    pub fn to_config(&self) -> Ddr5Config {
        let raw = &self.raw;
        let bin = raw.speed_bin.bin();
        let mut timing = Ddr5Timing::from_bin(&bin);
        for (&field, &clocks) in &raw.timing {
            timing.set(field, clocks);
        }
        let ns_to_clocks = |ns: u64| Constraint::ps(ns * 1000).cycles(bin.data_rate_mts);
        let power_down = raw.power_down_idle_ns.map_or(PowerDownPolicy::Disabled, |ns| {
            PowerDownPolicy::AfterIdle { idle_clocks: ns_to_clocks(ns) }
        });
        let ecc = match raw.ecc {
            EccMode::None => EccKind::None,
            EccMode::SecDed => EccKind::SecDed { patrol_scrub_ns: raw.patrol_scrub_ns },
            EccMode::ChipKill => EccKind::ChipKill { patrol_scrub_ns: raw.patrol_scrub_ns },
        };
        Ddr5Config {
            channels: raw.channels,
            subchannels_per_channel: raw.subchannels_per_channel,
            ranks_per_channel: raw.ranks_per_channel,
            bank_groups_per_rank: raw.bank_groups_per_rank,
            banks_per_group: raw.banks_per_group,
            row_bits: raw.row_bits,
            column_bits: raw.column_bits,
            row_size_bytes: 1u32 << (u32::from(raw.column_bits) + 6),
            read_queue_entries: raw.read_queue_entries,
            write_queue_entries: raw.write_queue_entries,
            write_high_watermark: raw.write_high_watermark,
            write_low_watermark: raw.write_low_watermark,
            min_writes_per_switch: raw.min_writes_per_switch,
            frontend_latency: ns_to_clocks(raw.frontend_latency_ns),
            backend_latency: ns_to_clocks(raw.backend_latency_ns),
            scheduler: raw.scheduler,
            refresh: raw.refresh,
            power_down,
            ecc,
            address_mapping: raw.address_mapping,
            timing,
        }
    }
}

/// The on-disk shape of [`Ddr5Params`], before validation.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Ddr5ParamsRaw {
    /// JEDEC speed bin the timing table is derived from.
    pub speed_bin: Ddr5SpeedBinName,
    /// Independent DRAM channels (power of two).
    pub channels: u8,
    /// Sub-channels per channel (DDR5: 2).
    pub subchannels_per_channel: u8,
    /// Ranks per channel (power of two).
    pub ranks_per_channel: u8,
    /// Bank groups per rank (power of two).
    pub bank_groups_per_rank: u8,
    /// Banks per bank group (power of two).
    pub banks_per_group: u8,
    /// Row-address bits.
    pub row_bits: u8,
    /// Column-address bits, in cache-line units (row size is `64 << column_bits`).
    pub column_bits: u8,
    /// Read queue capacity per subchannel.
    pub read_queue_entries: usize,
    /// Write queue capacity per subchannel.
    pub write_queue_entries: usize,
    /// Write-queue depth that starts a write drain.
    pub write_high_watermark: usize,
    /// Write-queue depth that ends a write drain.
    pub write_low_watermark: usize,
    /// Minimum writes issued per drain.
    pub min_writes_per_switch: usize,
    /// Fixed front-end controller latency in nanoseconds.
    pub frontend_latency_ns: u64,
    /// Fixed back-end controller latency in nanoseconds.
    pub backend_latency_ns: u64,
    /// Request selection policy.
    pub scheduler: SchedulerKind,
    /// Refresh cadence policy.
    pub refresh: RefreshKind,
    /// Address-bit interleave.
    pub address_mapping: AddressMappingKind,
    /// Idle nanoseconds before a rank powers down; `None` disables power-down.
    pub power_down_idle_ns: Option<u64>,
    /// ECC mode.
    pub ecc: EccMode,
    /// Nanoseconds between patrol-scrub reads; `None` disables scrubbing.
    pub patrol_scrub_ns: Option<u64>,
    /// Per-field overrides of the resolved timing table, in command clocks.
    pub timing: BTreeMap<Ddr5TimingField, u64>,
}

impl Default for Ddr5ParamsRaw {
    fn default() -> Self {
        let base = Ddr5Config::ddr5_4800();
        Self {
            speed_bin: Ddr5SpeedBinName::Ddr5_4800B,
            channels: base.channels,
            subchannels_per_channel: base.subchannels_per_channel,
            ranks_per_channel: base.ranks_per_channel,
            bank_groups_per_rank: base.bank_groups_per_rank,
            banks_per_group: base.banks_per_group,
            row_bits: base.row_bits,
            column_bits: base.column_bits,
            read_queue_entries: base.read_queue_entries,
            write_queue_entries: base.write_queue_entries,
            write_high_watermark: base.write_high_watermark,
            write_low_watermark: base.write_low_watermark,
            min_writes_per_switch: base.min_writes_per_switch,
            frontend_latency_ns: 10,
            backend_latency_ns: 10,
            scheduler: base.scheduler,
            refresh: base.refresh,
            address_mapping: base.address_mapping,
            power_down_idle_ns: None,
            ecc: EccMode::None,
            patrol_scrub_ns: None,
            timing: BTreeMap::new(),
        }
    }
}

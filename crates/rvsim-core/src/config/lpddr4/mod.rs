//! Configuration of LPDDR4 memory for the command-level DRAM controller:
//! speed bins and the parameter block a configuration names.
//!
//! An LPDDR4 package of x32 serves two independent 16-bit channels, each a
//! die of eight banks with 2 KiB rows and no bank groups, and moves a 64-byte
//! line in one 32-beat burst. Each channel has its own command and data bus,
//! so it is one channel here with a single sub-channel.

pub mod timing;

use std::collections::BTreeMap;
use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

use crate::config::AddressMappingKind;
use crate::config::dram::timing::{Constraint, DramTimingField};
use crate::config::dram::{
    CACHE_LINE_OFFSET_BITS, DrainQueues, DramParamsError, DramStandardConfig, EccKind,
    PowerDownPolicy, RefreshKind, SchedulerKind, check_topology,
};
pub use timing::{LPDDR4_COMMAND_CYCLES, Lpddr4Density, Lpddr4SpeedBin};

/// The parameter block's name in configuration and its errors.
const STANDARD: &str = "lpddr4";

/// Banks in an LPDDR4 channel.
const BANKS_PER_CHANNEL: NonZeroU8 = match NonZeroU8::new(8) {
    Some(banks) => banks,
    None => panic!("a channel holds banks"),
};

/// Column bits of a 2 KiB row, in 64-byte lines.
const COLUMN_BITS: u8 = 5;

/// A JEDEC speed bin selectable from configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum Lpddr4SpeedBinName {
    /// LPDDR4-3200.
    #[default]
    Lpddr4_3200,
}

impl Lpddr4SpeedBinName {
    const fn bin(self) -> Lpddr4SpeedBin {
        match self {
            Self::Lpddr4_3200 => Lpddr4SpeedBin::LPDDR4_3200,
        }
    }
}

/// Which refresh command the controller issues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum Lpddr4Refresh {
    /// `REFab`: all eight banks every tREFI, for tRFCab.
    #[default]
    AllBank,
    /// `REFpb`: one bank every tREFI / 8, for tRFCpb, while the others serve.
    PerBank,
}

impl Lpddr4Refresh {
    /// The controller's policy: with one bank group, a same-bank refresh
    /// covers one bank, as `REFpb` does.
    const fn kind(self) -> RefreshKind {
        match self {
            Self::AllBank => RefreshKind::AllBank,
            Self::PerBank => RefreshKind::SameBank,
        }
    }
}

/// LPDDR4 parameters as they appear in the configuration file, validated
/// on deserialization (see [`DramParamsError`]) and resolved into a
/// [`DramStandardConfig`] by [`Lpddr4Params::to_config`].
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(try_from = "Lpddr4ParamsRaw", into = "Lpddr4ParamsRaw")]
pub struct Lpddr4Params {
    raw: Lpddr4ParamsRaw,
}

impl From<Lpddr4Params> for Lpddr4ParamsRaw {
    fn from(params: Lpddr4Params) -> Self {
        params.raw
    }
}

impl TryFrom<Lpddr4ParamsRaw> for Lpddr4Params {
    type Error = DramParamsError;

    fn try_from(raw: Lpddr4ParamsRaw) -> Result<Self, Self::Error> {
        check_topology(
            STANDARD,
            &[("channels", raw.channels), ("ranks_per_channel", raw.ranks_per_channel)],
        )?;
        DrainQueues {
            read_entries: raw.read_queue_entries,
            write_entries: raw.write_queue_entries,
            high_watermark: raw.write_high_watermark,
            low_watermark: raw.write_low_watermark,
        }
        .check(STANDARD)?;
        Ok(Self { raw })
    }
}

impl Lpddr4Params {
    /// Resolves the parameters into a controller configuration.
    #[must_use]
    pub fn to_config(&self) -> DramStandardConfig {
        let raw = &self.raw;
        let bin = raw.speed_bin.bin();
        let mut timing = bin.timing(raw.density);
        for (&field, &clocks) in &raw.timing {
            timing.set(field, clocks);
        }
        let ns_to_clocks = |ns: u64| Constraint::ps(ns * 1000).cycles(bin.data_rate_mts);
        let power_down = raw.power_down_idle_ns.map_or(PowerDownPolicy::Disabled, |ns| {
            PowerDownPolicy::AfterIdle { idle_clocks: ns_to_clocks(ns) }
        });
        DramStandardConfig {
            channels: raw.channels,
            subchannels_per_channel: 1,
            ranks_per_channel: raw.ranks_per_channel,
            bank_groups_per_rank: 1,
            banks_per_group: BANKS_PER_CHANNEL,
            row_bits: raw.density.row_bits(),
            column_bits: COLUMN_BITS,
            row_size_bytes: 1u32 << (u32::from(COLUMN_BITS) + u32::from(CACHE_LINE_OFFSET_BITS)),
            read_queue_entries: raw.read_queue_entries,
            write_queue_entries: raw.write_queue_entries,
            write_high_watermark: raw.write_high_watermark,
            write_low_watermark: raw.write_low_watermark,
            min_writes_per_switch: raw.min_writes_per_switch,
            frontend_latency: ns_to_clocks(raw.frontend_latency_ns),
            backend_latency: ns_to_clocks(raw.backend_latency_ns),
            scheduler: raw.scheduler,
            refresh: raw.refresh.kind(),
            power_down,
            ecc: EccKind::None,
            address_mapping: raw.address_mapping,
            timing,
            commands: LPDDR4_COMMAND_CYCLES,
        }
    }
}

/// The on-disk shape of [`Lpddr4Params`], before validation.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Lpddr4ParamsRaw {
    /// JEDEC speed bin the timing table is derived from.
    pub speed_bin: Lpddr4SpeedBinName,
    /// Density of each channel's die.
    pub density: Lpddr4Density,
    /// Independent 16-bit channels (power of two); an x32 package has two.
    pub channels: u8,
    /// Ranks per channel (power of two).
    pub ranks_per_channel: u8,
    /// Read queue capacity per channel.
    pub read_queue_entries: usize,
    /// Write queue capacity per channel.
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
    /// Refresh command.
    pub refresh: Lpddr4Refresh,
    /// Address-bit interleave.
    pub address_mapping: AddressMappingKind,
    /// Idle nanoseconds before a rank powers down; `None` disables power-down.
    pub power_down_idle_ns: Option<u64>,
    /// Per-field overrides of the resolved timing table, in command clocks.
    pub timing: BTreeMap<DramTimingField, u64>,
}

impl Default for Lpddr4ParamsRaw {
    /// An x32 LPDDR4-3200 package of 16 Gb dies, one rank, with the DDR5
    /// controller's gem5-sized queues and latencies.
    fn default() -> Self {
        let ddr5 = crate::config::ddr5::ddr5_4800();
        Self {
            speed_bin: Lpddr4SpeedBinName::Lpddr4_3200,
            density: Lpddr4Density::Gb16,
            channels: 2,
            ranks_per_channel: 1,
            read_queue_entries: ddr5.read_queue_entries,
            write_queue_entries: ddr5.write_queue_entries,
            write_high_watermark: ddr5.write_high_watermark,
            write_low_watermark: ddr5.write_low_watermark,
            min_writes_per_switch: ddr5.min_writes_per_switch,
            frontend_latency_ns: 10,
            backend_latency_ns: 10,
            scheduler: SchedulerKind::FrFcfs,
            refresh: Lpddr4Refresh::AllBank,
            address_mapping: AddressMappingKind::default(),
            power_down_idle_ns: None,
            timing: BTreeMap::new(),
        }
    }
}

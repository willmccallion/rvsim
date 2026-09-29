//! Statistics paths and publication for the DDR5 controller.
//!
//! Counters accumulate inline in the DRAM state ([`SubchannelCounters`],
//! [`BankCounters`]) while commands issue, and are published to the
//! simulator's [`Stats`] tree once per controller tick. Paths follow
//! `memctrl<N>.ch<C>.sc<S>.<counter>` for the subchannel level and
//! `memctrl<N>.ch<C>.sc<S>.rank<R>.bank<B>.<counter>` for banks. The
//! subchannel counters are registered so they appear in the summary; bank
//! counters stay unregistered (queryable, but not summarised) to keep the
//! report readable.

use crate::sim::stats::StatId;

use crate::sim::stats::{Formula, Meta, Stats};
use crate::soc::memory::ddr5::state::{Bank, Subchannel};

/// Counters for one subchannel.
#[derive(Clone, Debug, Default)]
pub struct SubchannelCounters {
    /// Demand reads admitted to the read queue.
    pub reads: u64,
    /// Writes admitted to the write queue.
    pub writes: u64,
    /// Writes merged into a queued write to the same line.
    pub writes_merged: u64,
    /// Reads answered from the write queue.
    pub reads_hit_write_queue: u64,
    /// Patrol-scrub reads issued.
    pub scrub_reads: u64,
    /// ACTIVATE commands.
    pub activates: u64,
    /// Single-bank PRECHARGE commands.
    pub precharges: u64,
    /// PRECHARGE-ALL commands issued ahead of a refresh.
    pub precharge_alls: u64,
    /// REFRESH commands.
    pub refreshes: u64,
    /// Column commands that found their row open.
    pub row_hits: u64,
    /// Column commands that needed an ACTIVATE first.
    pub row_misses: u64,
    /// Power-down entries.
    pub power_down_entries: u64,
    /// Power-down exits.
    pub power_down_exits: u64,
    /// Clocks the data bus carried a burst.
    pub bus_busy_clocks: u64,
    /// DRAM clocks elapsed.
    pub clocks: u64,
    /// Clocks an arrived read waited for a read-queue slot.
    pub read_admission_stalls: u64,
    /// Clocks an arrived write waited for a write-queue slot.
    pub write_admission_stalls: u64,
    /// Arrival-to-burst-end latency of each demand read, in DRAM clocks.
    pub read_latency_samples: Vec<u64>,
    /// Read-queue occupancy seen by each admitted read.
    pub read_queue_depth_samples: Vec<u64>,
    /// Write-queue occupancy seen by each admitted write.
    pub write_queue_depth_samples: Vec<u64>,
}

/// Counters for one bank.
#[derive(Clone, Copy, Debug, Default)]
pub struct BankCounters {
    /// ACTIVATE commands.
    pub activates: u64,
    /// READ commands.
    pub reads: u64,
    /// WRITE commands.
    pub writes: u64,
    /// Column commands that found the row open.
    pub row_hits: u64,
    /// Column commands that needed an ACTIVATE first.
    pub row_misses: u64,
}

impl BankCounters {
    /// All-zero counters.
    #[must_use]
    pub const fn new() -> Self {
        Self { activates: 0, reads: 0, writes: 0, row_hits: 0, row_misses: 0 }
    }
}

/// Leaked static path strings for one controller's stats.
#[derive(Debug)]
pub struct ControllerStatPaths {
    subchannels: Vec<SubchannelPaths>,
    subchannels_per_channel: usize,
    banks_per_rank: usize,
}

#[derive(Debug)]
struct SubchannelPaths {
    reads: StatId,
    writes: StatId,
    writes_merged: StatId,
    reads_hit_write_queue: StatId,
    scrub_reads: StatId,
    activates: StatId,
    precharges: StatId,
    precharge_alls: StatId,
    refreshes: StatId,
    row_hits: StatId,
    row_misses: StatId,
    row_hit_rate: StatId,
    power_down_entries: StatId,
    power_down_exits: StatId,
    bus_busy_clocks: StatId,
    clocks: StatId,
    data_bus_utilization: StatId,
    read_admission_stalls: StatId,
    write_admission_stalls: StatId,
    read_latency: StatId,
    read_queue_depth: StatId,
    write_queue_depth: StatId,
    banks: Vec<BankPaths>,
}

#[derive(Debug)]
struct BankPaths {
    activates: StatId,
    reads: StatId,
    writes: StatId,
    row_hits: StatId,
    row_misses: StatId,
}

impl ControllerStatPaths {
    /// Builds the path table for controller `index` with the given topology.
    #[must_use]
    pub fn new(
        index: u32,
        channels: usize,
        subchannels_per_channel: usize,
        ranks: usize,
        banks_per_rank: usize,
    ) -> Self {
        let mut subchannels = Vec::with_capacity(channels * subchannels_per_channel);
        for ch in 0..channels {
            for sc in 0..subchannels_per_channel {
                let prefix = format!("memctrl{index}.ch{ch}.sc{sc}");
                let mut banks = Vec::with_capacity(ranks * banks_per_rank);
                for rank in 0..ranks {
                    for bank in 0..banks_per_rank {
                        let bank_prefix = format!("{prefix}.rank{rank}.bank{bank}");
                        banks.push(BankPaths {
                            activates: StatId::of(&format!("{bank_prefix}.activates")),
                            reads: StatId::of(&format!("{bank_prefix}.reads")),
                            writes: StatId::of(&format!("{bank_prefix}.writes")),
                            row_hits: StatId::of(&format!("{bank_prefix}.row_hits")),
                            row_misses: StatId::of(&format!("{bank_prefix}.row_misses")),
                        });
                    }
                }
                subchannels.push(SubchannelPaths {
                    reads: StatId::of(&format!("{prefix}.reads")),
                    writes: StatId::of(&format!("{prefix}.writes")),
                    writes_merged: StatId::of(&format!("{prefix}.writes_merged")),
                    reads_hit_write_queue: StatId::of(&format!("{prefix}.reads_hit_write_queue")),
                    scrub_reads: StatId::of(&format!("{prefix}.scrub_reads")),
                    activates: StatId::of(&format!("{prefix}.activates")),
                    precharges: StatId::of(&format!("{prefix}.precharges")),
                    precharge_alls: StatId::of(&format!("{prefix}.precharge_alls")),
                    refreshes: StatId::of(&format!("{prefix}.refreshes")),
                    row_hits: StatId::of(&format!("{prefix}.row_hits")),
                    row_misses: StatId::of(&format!("{prefix}.row_misses")),
                    row_hit_rate: StatId::of(&format!("{prefix}.row_hit_rate")),
                    power_down_entries: StatId::of(&format!("{prefix}.power_down_entries")),
                    power_down_exits: StatId::of(&format!("{prefix}.power_down_exits")),
                    bus_busy_clocks: StatId::of(&format!("{prefix}.bus_busy_clocks")),
                    clocks: StatId::of(&format!("{prefix}.clocks")),
                    data_bus_utilization: StatId::of(&format!("{prefix}.data_bus_utilization")),
                    read_admission_stalls: StatId::of(&format!("{prefix}.read_admission_stalls")),
                    write_admission_stalls: StatId::of(&format!("{prefix}.write_admission_stalls")),
                    read_latency: StatId::of(&format!("{prefix}.read_latency")),
                    read_queue_depth: StatId::of(&format!("{prefix}.read_queue_depth")),
                    write_queue_depth: StatId::of(&format!("{prefix}.write_queue_depth")),
                    banks,
                });
            }
        }
        Self { subchannels, subchannels_per_channel, banks_per_rank }
    }

    /// Registers metadata and derived stats for every subchannel.
    pub fn register(&self, stats: &mut Stats) {
        for sc in &self.subchannels {
            stats.register(sc.reads, Meta::events("demand reads admitted"));
            stats.register(sc.writes, Meta::events("writes admitted"));
            stats.register(sc.writes_merged, Meta::events("writes merged into a queued write"));
            stats.register(
                sc.reads_hit_write_queue,
                Meta::events("reads served from the write queue"),
            );
            stats.register(sc.scrub_reads, Meta::events("patrol-scrub reads"));
            stats.register(sc.activates, Meta::events("ACTIVATE commands"));
            stats.register(sc.precharges, Meta::events("PRECHARGE commands"));
            stats.register(sc.precharge_alls, Meta::events("PRECHARGE-ALL commands"));
            stats.register(sc.refreshes, Meta::events("REFRESH commands"));
            stats.register(sc.row_hits, Meta::events("column commands hitting an open row"));
            stats.register(sc.row_misses, Meta::events("column commands needing an ACTIVATE"));
            stats.register(sc.power_down_entries, Meta::events("power-down entries"));
            stats.register(sc.power_down_exits, Meta::events("power-down exits"));
            stats.register(
                sc.bus_busy_clocks,
                Meta::cycles("DRAM clocks with a burst on the data bus"),
            );
            stats.register(sc.clocks, Meta::cycles("DRAM clocks elapsed"));
            stats.register(
                sc.read_admission_stalls,
                Meta::cycles("clocks a read waited for a queue slot"),
            );
            stats.register(
                sc.write_admission_stalls,
                Meta::cycles("clocks a write waited for a queue slot"),
            );
            stats.derive(
                sc.row_hit_rate,
                Formula::Ratio { numerator: sc.row_hits, other: sc.row_misses },
                Meta::ratio("row-buffer hit rate"),
            );
            stats.derive(
                sc.data_bus_utilization,
                Formula::Div(sc.bus_busy_clocks, sc.clocks),
                Meta::ratio("fraction of DRAM clocks the data bus was busy"),
            );
        }
    }

    /// Adds `subchannel`'s accumulated counters and samples to `stats` and
    /// clears them.
    pub fn publish(
        &self,
        stats: &mut Stats,
        channel: usize,
        subchannel_index: usize,
        subchannel: &mut Subchannel,
    ) {
        let paths = &self.subchannels[channel * self.subchannels_per_channel + subchannel_index];
        let c = &mut subchannel.counters;
        stats.counter(paths.reads).add(c.reads);
        stats.counter(paths.writes).add(c.writes);
        stats.counter(paths.writes_merged).add(c.writes_merged);
        stats.counter(paths.reads_hit_write_queue).add(c.reads_hit_write_queue);
        stats.counter(paths.scrub_reads).add(c.scrub_reads);
        stats.counter(paths.activates).add(c.activates);
        stats.counter(paths.precharges).add(c.precharges);
        stats.counter(paths.precharge_alls).add(c.precharge_alls);
        stats.counter(paths.refreshes).add(c.refreshes);
        stats.counter(paths.row_hits).add(c.row_hits);
        stats.counter(paths.row_misses).add(c.row_misses);
        stats.counter(paths.power_down_entries).add(c.power_down_entries);
        stats.counter(paths.power_down_exits).add(c.power_down_exits);
        stats.counter(paths.bus_busy_clocks).add(c.bus_busy_clocks);
        stats.counter(paths.clocks).add(c.clocks);
        stats.counter(paths.read_admission_stalls).add(c.read_admission_stalls);
        stats.counter(paths.write_admission_stalls).add(c.write_admission_stalls);
        for sample in c.read_latency_samples.drain(..) {
            stats.histogram(paths.read_latency).record(sample);
        }
        for sample in c.read_queue_depth_samples.drain(..) {
            stats.histogram(paths.read_queue_depth).record(sample);
        }
        for sample in c.write_queue_depth_samples.drain(..) {
            stats.histogram(paths.write_queue_depth).record(sample);
        }
        let had_bank_activity = c.activates + c.row_hits + c.row_misses > 0;
        *c = SubchannelCounters::default();
        if !had_bank_activity {
            return;
        }
        for (rank_index, rank) in subchannel.ranks.iter_mut().enumerate() {
            for (bank_index, bank) in rank.banks.iter_mut().enumerate() {
                let bank_paths = &paths.banks[rank_index * self.banks_per_rank + bank_index];
                publish_bank(stats, bank_paths, bank);
            }
        }
    }
}

fn publish_bank(stats: &mut Stats, paths: &BankPaths, bank: &mut Bank) {
    let c = bank.counters;
    if c.activates + c.reads + c.writes == 0 {
        return;
    }
    stats.counter(paths.activates).add(c.activates);
    stats.counter(paths.reads).add(c.reads);
    stats.counter(paths.writes).add(c.writes);
    stats.counter(paths.row_hits).add(c.row_hits);
    stats.counter(paths.row_misses).add(c.row_misses);
    bank.counters = BankCounters::default();
}

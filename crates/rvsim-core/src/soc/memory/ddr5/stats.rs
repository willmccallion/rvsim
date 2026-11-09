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

use crate::sim::stats::{Formula, Stats, Unit};
use crate::soc::memory::ddr5::state::{Bank, Subchannel};

/// Counters for one subchannel.
#[derive(Clone, Copy, Debug, Default)]
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
}

/// Histogram samples for one subchannel, kept between publications so
/// their buffers are reused.
#[derive(Clone, Debug, Default)]
pub struct SubchannelSamples {
    /// Arrival-to-burst-end latency of each demand read, in DRAM clocks.
    pub read_latency: Vec<u64>,
    /// Read-queue occupancy seen by each admitted read.
    pub read_queue_depth: Vec<u64>,
    /// Write-queue occupancy seen by each admitted write.
    pub write_queue_depth: Vec<u64>,
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
    subchannels: Vec<SubchannelStats>,
    subchannels_per_channel: usize,
    banks_per_rank: usize,
}

crate::sim::stats::paths::stat_paths! {
    /// One subchannel's paths, under `memctrl<N>.ch<C>.sc<S>`.
    SubchannelPaths {
        /// Demand reads admitted to the read queue.
        reads: "reads" => Events,
        /// Writes admitted to the write queue.
        writes: "writes" => Events,
        /// Writes merged into a queued write to the same line.
        writes_merged: "writes_merged" => Events,
        /// Reads answered from the write queue.
        reads_hit_write_queue: "reads_hit_write_queue" => Events,
        /// Patrol-scrub reads issued.
        scrub_reads: "scrub_reads" => Events,
        /// ACTIVATE commands.
        activates: "activates" => Events,
        /// Single-bank PRECHARGE commands.
        precharges: "precharges" => Events,
        /// PRECHARGE-ALL commands issued ahead of a refresh.
        precharge_alls: "precharge_alls" => Events,
        /// REFRESH commands.
        refreshes: "refreshes" => Events,
        /// Column commands that found their row open.
        row_hits: "row_hits" => Events,
        /// Column commands that needed an ACTIVATE first.
        row_misses: "row_misses" => Events,
        /// Power-down entries.
        power_down_entries: "power_down_entries" => Events,
        /// Power-down exits.
        power_down_exits: "power_down_exits" => Events,
        /// Clocks the data bus carried a burst.
        bus_busy_clocks: "bus_busy_clocks" => Cycles,
        /// DRAM clocks elapsed.
        clocks: "clocks" => Cycles,
        /// Clocks an arrived read waited for a read-queue slot.
        read_admission_stalls: "read_admission_stalls" => Cycles,
        /// Clocks an arrived write waited for a write-queue slot.
        write_admission_stalls: "write_admission_stalls" => Cycles,
    }
    histograms {
        /// Cycles from a read's arrival to its data.
        read_latency: "read_latency" => Cycles,
        /// Read-queue occupancy, sampled each clock.
        read_queue_depth: "read_queue_depth" => Events,
        /// Write-queue occupancy, sampled each clock.
        write_queue_depth: "write_queue_depth" => Events,
    }
    derived {
        /// Row hits over row hits and misses.
        row_hit_rate: "row_hit_rate",
        /// Bus-busy clocks over all clocks.
        data_bus_utilization: "data_bus_utilization",
    }
}

/// One subchannel's paths and its banks', in rank-major order.
#[derive(Debug)]
struct SubchannelStats {
    paths: SubchannelPaths,
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
                let paths = SubchannelPaths::under(&prefix);
                subchannels.push(SubchannelStats { paths, banks });
            }
        }
        Self { subchannels, subchannels_per_channel, banks_per_rank }
    }

    /// Registers metadata and derived stats for every subchannel.
    pub fn register(&self, stats: &mut Stats) {
        for subchannel in &self.subchannels {
            let sc = &subchannel.paths;
            sc.register_counters(stats);
            stats.derive(
                sc.row_hit_rate,
                Formula::Ratio { numerator: sc.row_hits, other: sc.row_misses },
                Unit::Ratio,
            );
            stats.derive(
                sc.data_bus_utilization,
                Formula::Div(sc.bus_busy_clocks, sc.clocks),
                Unit::Ratio,
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
        let entry = &self.subchannels[channel * self.subchannels_per_channel + subchannel_index];
        let paths = &entry.paths;
        let c = std::mem::take(&mut subchannel.counters);
        add_counts(stats, paths.reads, c.reads);
        add_counts(stats, paths.writes, c.writes);
        add_counts(stats, paths.writes_merged, c.writes_merged);
        add_counts(stats, paths.reads_hit_write_queue, c.reads_hit_write_queue);
        add_counts(stats, paths.scrub_reads, c.scrub_reads);
        add_counts(stats, paths.activates, c.activates);
        add_counts(stats, paths.precharges, c.precharges);
        add_counts(stats, paths.precharge_alls, c.precharge_alls);
        add_counts(stats, paths.refreshes, c.refreshes);
        add_counts(stats, paths.row_hits, c.row_hits);
        add_counts(stats, paths.row_misses, c.row_misses);
        add_counts(stats, paths.power_down_entries, c.power_down_entries);
        add_counts(stats, paths.power_down_exits, c.power_down_exits);
        add_counts(stats, paths.bus_busy_clocks, c.bus_busy_clocks);
        add_counts(stats, paths.clocks, c.clocks);
        add_counts(stats, paths.read_admission_stalls, c.read_admission_stalls);
        add_counts(stats, paths.write_admission_stalls, c.write_admission_stalls);
        let samples = &mut subchannel.samples;
        for sample in samples.read_latency.drain(..) {
            stats.histogram(paths.read_latency).record(sample);
        }
        for sample in samples.read_queue_depth.drain(..) {
            stats.histogram(paths.read_queue_depth).record(sample);
        }
        for sample in samples.write_queue_depth.drain(..) {
            stats.histogram(paths.write_queue_depth).record(sample);
        }
        let had_bank_activity = c.activates + c.row_hits + c.row_misses > 0;
        if !had_bank_activity {
            return;
        }
        for (rank_index, rank) in subchannel.ranks.iter_mut().enumerate() {
            for (bank_index, bank) in rank.banks.iter_mut().enumerate() {
                let bank_paths = &entry.banks[rank_index * self.banks_per_rank + bank_index];
                publish_bank(stats, bank_paths, bank);
            }
        }
    }
}

/// Adds `count` to `stat`. Every subchannel counter is registered before
/// the first publication, so skipping a zero count changes nothing.
fn add_counts(stats: &mut Stats, stat: StatId, count: u64) {
    if count != 0 {
        stats.counter(stat).add(count);
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

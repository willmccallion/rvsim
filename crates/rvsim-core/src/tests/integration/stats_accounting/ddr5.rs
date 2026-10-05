//! The DDR5 controller's counts, summed over its subchannels: the requests
//! the caches send it, the column, row and bus traffic they become, its
//! queues, refresh, power-down and scrubbing.

use super::program::{
    A1, BACKENDS, T1, T2, config, ending_in_spin, run_to_pc, system_with_configured_latency,
};
use super::{Recorder, accounting_checks};
use crate::config::{BackendKind, Config, MemoryControllerKind};
use crate::soc::memory::ddr5::controller::ClockRatio;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

/// Cycles run after a program's end for every write to drain.
const SETTLE: u64 = 20_000;

/// One hart with a 1 KiB L1D, a 4 KiB L2 and a 16 KiB LLC in front of a
/// DDR5 controller with `params` (JSON, defaults for what it leaves out).
fn ddr5_system(backend: BackendKind, params: &str) -> Config {
    let mut config = config(backend);
    for (cache, bytes, ways) in [
        (&mut config.cache.l1_d, 1024, 1),
        (&mut config.cache.l2, 4096, 4),
        (&mut config.cache.l3, 16 * 1024, 8),
    ] {
        cache.enabled = true;
        cache.size_bytes = bytes;
        cache.ways = ways;
    }
    config.memory.controller = MemoryControllerKind::Ddr5;
    config.memory.ddr5 = serde_json::from_str(params).expect("DDR5 parameters");
    config
}

/// Reads then writes one word in each of `lines` lines from `a1`, then
/// spins.
fn read_modify_write_lines(lines: i32) -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().addi(T1, 0, lines).build(),
        i().ld(T2, A1, 0).build(),
        i().addi(T2, T2, 1).build(),
        i().sd(A1, T2, 0).build(),
        i().addi(A1, A1, 64).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -20).build(),
    ])
}

fn run_settled(config: &Config, (program, end): (Vec<u32>, u64), context: &str) -> TestContext {
    let mut ctx = system_with_configured_latency(config, &program, &[]);
    run_to_pc(&mut ctx, end, context);
    ctx.run(SETTLE);
    ctx
}

/// Every subchannel's path for `stat`.
fn subchannel_paths(ctx: &TestContext, stat: &str) -> Vec<String> {
    let query = ctx.sim.stats().query(&format!("memctrl0.ch*.sc*.{stat}"));
    let paths: Vec<String> = query.iter().map(|(path, _)| path.to_owned()).collect();
    assert!(!paths.is_empty(), "no subchannel counts {stat}");
    paths
}

/// `stat` summed over every subchannel.
fn total(rec: &mut Recorder, ctx: &TestContext, stat: &str) -> u64 {
    subchannel_paths(ctx, stat).iter().map(|path| rec.read(&ctx.sim, path)).sum()
}

fn llc(rec: &mut Recorder, ctx: &TestContext, stat: &str) -> u64 {
    rec.read(&ctx.sim, &format!("llc.{stat}"))
}

fn every_llc_fetch_and_writeback_reaches_dram_once(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&ddr5_system(backend, "{}"), read_modify_write_lines(512), &context);

        let fetched = llc(rec, &ctx, "misses") - llc(rec, &ctx, "mshr_hits")
            + llc(rec, &ctx, "prefetches.issued");
        let read = total(rec, &ctx, "reads") + total(rec, &ctx, "reads_hit_write_queue");
        assert_eq!(read, fetched, "{context}: every LLC fetch is a DRAM read");
        let written = total(rec, &ctx, "writes") + total(rec, &ctx, "writes_merged");
        assert_eq!(written, llc(rec, &ctx, "writebacks"), "{context}: every writeback a write");
        assert!(written > 0, "{context}: 512 dirty lines through a 16 KiB LLC");
    }
}

fn each_column_command_counts_its_row_and_its_burst(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let config = ddr5_system(backend, "{}");
        let burst = config.memory.ddr5.to_config().timing.bl_half;

        let ctx = run_settled(&config, read_modify_write_lines(512), &context);

        let (hits, misses) = (total(rec, &ctx, "row_hits"), total(rec, &ctx, "row_misses"));
        let columns = total(rec, &ctx, "reads")
            + total(rec, &ctx, "scrub_reads")
            + total(rec, &ctx, "writes");
        assert_eq!(hits + misses, columns, "{context}: every queued request issued once");
        assert_eq!(total(rec, &ctx, "activates"), misses, "{context}: a row miss activates");
        assert!(total(rec, &ctx, "precharges") <= misses, "{context}");
        assert_eq!(total(rec, &ctx, "bus_busy_clocks"), columns * burst, "{context}");
        for path in subchannel_paths(&ctx, "row_hits") {
            let subchannel = path.trim_end_matches(".row_hits");
            let stat = |rec: &mut Recorder, name: &str| {
                rec.read(&ctx.sim, &format!("{subchannel}.{name}"))
            };
            let (h, m) = (stat(rec, "row_hits"), stat(rec, "row_misses"));
            let rate = if h + m == 0 { 0.0 } else { h as f64 / (h + m) as f64 };
            rec.expect_ratio(&ctx.sim, &format!("{subchannel}.row_hit_rate"), rate, &context);
            let (busy, clocks) = (stat(rec, "bus_busy_clocks"), stat(rec, "clocks"));
            let used = busy as f64 / clocks as f64;
            rec.expect_ratio(
                &ctx.sim,
                &format!("{subchannel}.data_bus_utilization"),
                used,
                &context,
            );
        }
    }
}

fn the_latency_and_queue_samples_are_one_per_request(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&ddr5_system(backend, "{}"), read_modify_write_lines(512), &context);

        let samples = |rec: &mut Recorder, name: &str| -> u64 {
            subchannel_paths(&ctx, "reads")
                .iter()
                .map(|path| path.trim_end_matches(".reads").to_owned())
                .filter_map(|subchannel| {
                    let path = format!("{subchannel}.{name}");
                    ctx.sim
                        .stats()
                        .histogram_at(path.as_str())
                        .is_some()
                        .then(|| rec.histogram(&ctx.sim, &path).count())
                })
                .sum()
        };
        let reads = total(rec, &ctx, "reads");
        assert_eq!(samples(rec, "read_latency"), reads, "{context}: a latency per demand read");
        let queued = reads + total(rec, &ctx, "scrub_reads");
        assert_eq!(samples(rec, "read_queue_depth"), queued, "{context}");
        assert_eq!(samples(rec, "write_queue_depth"), total(rec, &ctx, "writes"), "{context}");
    }
}

fn every_dram_clock_is_counted_and_refreshed_on_time(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let config = ddr5_system(backend, "{}");
        let ddr5 = config.memory.ddr5.to_config();
        let clock = ClockRatio::new(config.system.cpu_clock_mhz, ddr5.timing.data_rate_mts);

        let ctx = run_settled(&config, read_modify_write_lines(64), &context);

        let elapsed = clock.to_dram(ctx.sim.cycle()) + 1;
        let subchannels = subchannel_paths(&ctx, "clocks");
        for path in &subchannels {
            rec.expect(&ctx.sim, path, elapsed, &context);
        }
        let ranks = u64::from(ddr5.ranks_per_channel) * subchannels.len() as u64;
        let due = elapsed / ddr5.timing.t_refi * ranks;
        let refreshes = total(rec, &ctx, "refreshes");
        assert!(
            (due.saturating_sub(ranks)..=due + ranks).contains(&refreshes),
            "{context}: {refreshes} of {due}"
        );
        assert!(total(rec, &ctx, "precharge_alls") <= refreshes, "{context}: before refreshes");
    }
}

fn idle_ranks_power_down_and_a_patrol_scrubs_when_configured(rec: &mut Recorder) {
    let configured = r#"{"power_down_idle_ns": 50, "ecc": "SecDed", "patrol_scrub_ns": 5000}"#;
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let plain = run_settled(&ddr5_system(backend, "{}"), read_modify_write_lines(64), &context);
        let both =
            run_settled(&ddr5_system(backend, configured), read_modify_write_lines(64), &context);

        for stat in ["power_down_entries", "power_down_exits", "scrub_reads"] {
            assert_eq!(total(rec, &plain, stat), 0, "{context}: {stat} unconfigured");
        }
        let (entries, exits) =
            (total(rec, &both, "power_down_entries"), total(rec, &both, "power_down_exits"));
        let ranks = subchannel_paths(&both, "clocks").len() as u64
            * u64::from(both.sim.state.config.memory.ddr5.to_config().ranks_per_channel);
        assert!(
            exits > 0 && (exits..=exits + ranks).contains(&entries),
            "{context}: {entries} in, {exits} out"
        );
        assert!(total(rec, &both, "scrub_reads") > 0, "{context}");
    }
}

fn requests_wait_for_a_queue_slot_only_when_the_queues_are_full(rec: &mut Recorder) {
    let tiny = r#"{"read_queue_entries": 2, "write_queue_entries": 4, "write_high_watermark": 3, "write_low_watermark": 1}"#;
    let context = "OutOfOrder width 4";
    let mut stalls = [[0; 2]; 2];
    for (n, params) in ["{}", tiny].into_iter().enumerate() {
        let mut config = ddr5_system(BackendKind::OutOfOrder, params);
        config.pipeline.width = 4;
        config.cache.l3.prefetcher = crate::config::PrefetcherKind::NextLine;

        let ctx = run_settled(&config, read_modify_write_lines(512), context);

        stalls[n] =
            [total(rec, &ctx, "read_admission_stalls"), total(rec, &ctx, "write_admission_stalls")];
    }
    assert!(stalls[1][0] > 0 && stalls[1][1] > 0, "{context}: tiny queues: {stalls:?}");
    assert!(stalls[0][0] < stalls[1][0] && stalls[0][1] < stalls[1][1], "{context}: {stalls:?}");
}

accounting_checks!(
    every_llc_fetch_and_writeback_reaches_dram_once,
    each_column_command_counts_its_row_and_its_burst,
    the_latency_and_queue_samples_are_one_per_request,
    every_dram_clock_is_counted_and_refreshed_on_time,
    idle_ranks_power_down_and_a_patrol_scrubs_when_configured,
    requests_wait_for_a_queue_slot_only_when_the_queues_are_full,
);

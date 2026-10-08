//! Every cache level's counts: what its requests found, what it fetched,
//! held and evicted, what it wrote back and dropped, and its prefetches.
//! Coherence traffic, which needs several cores, is checked with the rest
//! of the coherence stats.

use super::program::{
    A1, BACKENDS, PROGRAM_BASE, T0, T1, T2, config, ending_in_spin, run_to_pc, system_with,
};
use super::{Recorder, accounting_checks};
use crate::config::{
    BackendKind, CacheConfig, Config, InclusionPolicy, PrefetcherKind, StorePrefetcherConfig,
};
use crate::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
use crate::isa::encoding::zicboz::CBO_CLEAN_IMM;
use crate::tests::support::builder::instruction::{FENCE_I, FENCE_IORW, InstructionBuilder};
use crate::tests::support::count;
use crate::tests::support::harness::TestContext;

const CACHES: [&str; 4] = ["core0.cache.l1i", "core0.cache.l1d", "core0.cache.l2", "llc"];
/// Cycles run after a program's end for every fetch it started to fill.
const SETTLE: u64 = 3_000;

fn sized(cache: &mut CacheConfig, bytes: usize, ways: usize) {
    cache.enabled = true;
    cache.size_bytes = bytes;
    cache.ways = ways;
}

/// Every level on: a 1 KiB direct-mapped L1D, 2 KiB two-way L1I, 4 KiB
/// four-way L2 and 16 KiB eight-way LLC.
fn hierarchy(backend: BackendKind) -> Config {
    let mut config = config(backend);
    sized(&mut config.cache.l1_i, 2048, 2);
    sized(&mut config.cache.l1_d, 1024, 1);
    sized(&mut config.cache.l2, 4096, 4);
    sized(&mut config.cache.l3, 16 * 1024, 8);
    config
}

/// `hierarchy` with a next-line prefetcher at every level and the L1D's
/// store-miss prefetcher.
fn prefetching_hierarchy(backend: BackendKind) -> Config {
    let mut config = hierarchy(backend);
    for cache in
        [&mut config.cache.l1_i, &mut config.cache.l1_d, &mut config.cache.l2, &mut config.cache.l3]
    {
        cache.prefetcher = PrefetcherKind::NextLine;
    }
    config.cache.store_prefetcher = StorePrefetcherConfig::Stream { streams: 4, l2_lines: 2 };
    config
}

/// Reads then writes one word in each of `lines` lines from `a1`, each
/// load's address depending on the last load's (zero) value, then spins.
fn read_modify_write_lines(lines: i32) -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().addi(T1, 0, lines).build(),
        i().ld(T2, A1, 0).build(),
        i().add(A1, A1, T2).build(),
        i().addi(T2, T2, 1).build(),
        i().sd(A1, T2, 0).build(),
        i().addi(A1, A1, 64).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -24).build(),
    ])
}

/// Runs `program` to its spin, then on until every fetch has filled.
fn run_settled(config: &Config, (program, end): (Vec<u32>, u64), context: &str) -> TestContext {
    let mut ctx = system_with(config, &program, &[]);
    run_to_pc(&mut ctx, end, context);
    ctx.run(SETTLE);
    ctx
}

fn held_lines(ctx: &TestContext, cache: &str) -> u64 {
    let units = &ctx.sim.state.cores[0].units;
    let held = match cache {
        "core0.cache.l1i" => units.l1_i_cache.held_lines(),
        "core0.cache.l1d" => units.l1_d_cache.held_lines(),
        "core0.cache.l2" => units.l2_cache.held_lines(),
        _ => ctx.sim.state.uncore.l3_cache.held_lines(),
    };
    held.len() as u64
}

/// The counts every cache keeps consistent once nothing is in flight: each
/// fetch it started filled once, what it still holds is what it filled
/// less what it evicted, and its derived stats follow their formulas.
/// Returns the fetches it sent to the next level.
fn check_settled_cache(rec: &mut Recorder, ctx: &TestContext, cache: &str, context: &str) -> u64 {
    let sim = &ctx.sim;
    let mut stat = |name: &str| rec.read(sim, &format!("{cache}.{name}"));
    let (hits, misses, mshr_hits) = (stat("hits"), stat("misses"), stat("mshr_hits"));
    let (fills, evictions) = (stat("fills"), stat("evictions"));
    let issued = stat("prefetches.issued");
    let (late, useful, unused) =
        (stat("prefetches.late"), stat("prefetches.useful"), stat("prefetches.unused"));
    let used = stat("prefetches.used");
    let fetches = misses - mshr_hits + issued;
    assert_eq!(fills, fetches, "{context}: {cache}: every fetch fills once");
    assert_eq!(fills - evictions, held_lines(ctx, cache), "{context}: {cache}: lines held");
    assert_eq!(used, late + useful, "{context}: {cache}");
    assert!(late + useful + unused <= issued, "{context}: {cache}: prefetches resolved once");
    let accuracy = if issued == 0 { 0.0 } else { used as f64 / issued as f64 };
    rec.expect_ratio(sim, &format!("{cache}.prefetches.accuracy"), accuracy, context);
    let rate = if hits + misses == 0 { 0.0 } else { misses as f64 / (hits + misses) as f64 };
    rec.expect_ratio(sim, &format!("{cache}.miss_rate"), rate, context);
    fetches
}

/// Each level's requests are the fetches of the level above it.
fn check_hierarchy(rec: &mut Recorder, ctx: &TestContext, context: &str) {
    let [l1i, l1d, l2, _] = CACHES.map(|cache| check_settled_cache(rec, ctx, cache, context));
    let requests = |rec: &mut Recorder, cache: &str| {
        rec.read(&ctx.sim, &format!("{cache}.hits"))
            + rec.read(&ctx.sim, &format!("{cache}.misses"))
    };
    assert_eq!(requests(rec, "core0.cache.l2"), l1i + l1d, "{context}: the L2's requests");
    assert_eq!(requests(rec, "llc"), l2, "{context}: the LLC's requests");
}

fn every_level_fills_each_fetch_once_and_holds_what_it_did_not_evict(rec: &mut Recorder) {
    for backend in BACKENDS {
        for (config, label) in [
            (hierarchy(backend), "no prefetching"),
            (prefetching_hierarchy(backend), "prefetching"),
        ] {
            let context = format!("{backend:?} {label}");

            let ctx = run_settled(&config, read_modify_write_lines(48), &context);

            check_hierarchy(rec, &ctx, &context);
            let evicted = rec.read(&ctx.sim, "core0.cache.l1d.evictions");
            assert!(evicted > 0, "{context}: 48 lines through a 16-line L1D");
        }
    }
}

/// Loads two words of one line, then one of the next, then the first line
/// again, each address depending on the last load's (zero) value.
fn two_lines() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    for offset in [0, 8, 64, 0] {
        program.push(i().ld(T2, A1, offset).build());
        program.push(i().add(A1, A1, T2).build());
    }
    ending_in_spin(program)
}

fn a_cold_line_misses_once_and_then_hits(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&hierarchy(backend), two_lines(), &context);

        rec.expect(&ctx.sim, "core0.cache.l1d.misses", 2, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.hits", 2, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.mshr_hits", 0, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.fills", 2, &context);
    }
}

/// Writes a line, then loads the line a whole L1D away, which takes its
/// place, then spins.
fn dirty_conflict() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().addi(T0, 0, 5).build(),
        i().sd(A1, T0, 0).build(),
        i().lui(T1, 0).build(),
        i().addi(T1, T1, 1024).build(),
        i().add(T1, A1, T1).build(),
        i().ld(T2, T1, 0).build(),
    ])
}

fn a_dirty_victim_is_evicted_and_written_back(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&hierarchy(backend), dirty_conflict(), &context);

        rec.expect(&ctx.sim, "core0.cache.l1d.evictions", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.writebacks", 1, &context);
    }
}

/// Where the two conflicting lines start: clear of the L2 sets the code
/// occupies.
const CONFLICT_OFFSET: i32 = 0x200;

/// Loads a line, then the line a whole L2 away: the direct-mapped L2 can
/// hold only one of them, the four-way L1D both.
fn l2_conflict() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().ld(T2, A1, CONFLICT_OFFSET).build(),
        i().lui(T1, 1).build(),
        i().add(T1, A1, T1).build(),
        i().add(T1, T1, T2).build(),
        i().ld(T2, T1, CONFLICT_OFFSET).build(),
    ])
}

fn an_inclusive_l2_eviction_drops_the_l1_copy(rec: &mut Recorder) {
    for backend in BACKENDS {
        for (policy, dropped) in [(InclusionPolicy::Inclusive, 1), (InclusionPolicy::Nine, 0)] {
            let context = format!("{backend:?} {policy:?}");
            let mut config = hierarchy(backend);
            sized(&mut config.cache.l1_d, 2048, 4);
            sized(&mut config.cache.l2, 4096, 1);
            config.cache.inclusion_policy = policy;

            let ctx = run_settled(&config, l2_conflict(), &context);

            rec.expect(&ctx.sim, "core0.cache.l1d.back_invalidations", dropped, &context);
            rec.expect(&ctx.sim, "core0.cache.l2.evictions", 1, &context);
        }
    }
}

/// Loads line X, then Y a whole L1D away (which takes X's place there),
/// then Z a whole L2 away from X (which takes its place there): the L2 tells
/// the L1D to drop X, which it no longer holds.
fn back_invalidation_of_a_dropped_line() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().ld(T2, A1, CONFLICT_OFFSET).build(),
        i().add(A1, A1, T2).build(),
        i().ld(T2, A1, CONFLICT_OFFSET + 1024).build(),
        i().lui(T1, 1).build(),
        i().add(T1, A1, T1).build(),
        i().add(T1, T1, T2).build(),
        i().ld(T2, T1, CONFLICT_OFFSET).build(),
    ])
}

fn an_inclusive_llc_eviction_drops_the_l2_copy(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = hierarchy(backend);
        sized(&mut config.cache.l3, 4096, 1);
        config.cache.inclusion_policy = InclusionPolicy::Inclusive;

        let ctx = run_settled(&config, l2_conflict(), &context);

        rec.expect(&ctx.sim, "llc.evictions", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l2.back_invalidations", 1, &context);
    }
}

fn a_back_invalidation_for_a_line_already_gone_drops_nothing(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = hierarchy(backend);
        sized(&mut config.cache.l2, 4096, 1);
        config.cache.inclusion_policy = InclusionPolicy::Inclusive;

        let ctx = run_settled(&config, back_invalidation_of_a_dropped_line(), &context);

        rec.expect(&ctx.sim, "core0.cache.l2.evictions", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.back_invalidations", 0, &context);
    }
}

fn a_cbo_passes_through_and_counts_at_every_level(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let cbo_clean =
        ((CBO_CLEAN_IMM as u32 & 0xFFF) << 20) | (A1 << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM;
    let program = ending_in_spin(vec![
        i().addi(T0, 0, 5).build(),
        i().sd(A1, T0, 0).build(),
        cbo_clean,
        FENCE_IORW,
    ]);
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&hierarchy(backend), program.clone(), &context);

        for cache in ["core0.cache.l1d", "core0.cache.l2", "llc"] {
            rec.expect(&ctx.sim, &format!("{cache}.maintenance"), 1, &context);
        }
        rec.expect(&ctx.sim, "core0.cache.l1i.maintenance", 0, &context);
    }
}

/// Four loads from four lines, none depending on another.
fn independent_misses() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin((0..4).map(|n| i().ld(T0 + n % 3, A1, 64 * n as i32).build()).collect())
}

fn misses_beyond_the_mshrs_wait_as_blocked_requests(rec: &mut Recorder) {
    let context = "OutOfOrder width 4";
    let mut blocked = [0; 2];
    for (n, mshrs) in [2, 8].into_iter().enumerate() {
        let mut config = hierarchy(BackendKind::OutOfOrder);
        config.pipeline.width = 4;
        config.cache.l1_d.mshr_count = count(mshrs);

        let ctx = run_settled(&config, independent_misses(), context);

        blocked[n] = rec.read(&ctx.sim, "core0.cache.l1d.blocked_requests");
    }
    assert!(blocked[0] > 0, "{context}: two MSHRs: {blocked:?}");
    assert_eq!(blocked[1], 0, "{context}: eight MSHRs: {blocked:?}");
}

/// Loads the last line of `a1`'s page, then a line in the middle of it,
/// the second address depending on the first load's (zero) value.
fn page_end_then_middle() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().lui(T1, 1).build(),
        i().add(T1, A1, T1).build(),
        i().ld(T2, T1, -64).build(),
        i().add(A1, A1, T2).build(),
        i().ld(T2, A1, 512).build(),
    ])
}

fn a_next_line_candidate_past_the_page_is_dropped(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = hierarchy(backend);
        config.cache.l1_d.prefetcher = PrefetcherKind::NextLine;

        let ctx = run_settled(&config, page_end_then_middle(), &context);

        rec.expect(&ctx.sim, "core0.cache.l1d.prefetches.page_crossing", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.prefetches.issued", 1, &context);
    }
}

/// Stores to `lines` consecutive lines from `a1`.
fn store_run(lines: i32) -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin((0..lines).map(|n| i().sd(A1, T0, 64 * n).build()).collect())
}

fn store_prefetches_go_to_the_l2_and_are_dropped_without_an_mshr(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = hierarchy(backend);
        config.cache.store_prefetcher = StorePrefetcherConfig::Stream { streams: 4, l2_lines: 2 };
        config.cache.l2.mshr_count = count(16);
        let mut starved = config.clone();
        starved.cache.l2.mshr_count = count(1);

        let ctx = run_settled(&config, store_run(8), &context);
        let starved = run_settled(&starved, store_run(8), &context);

        let sent = rec.read(&ctx.sim, "core0.cache.l1d.prefetches.store_stream");
        assert!(sent > 0, "{context}");
        rec.expect(&ctx.sim, "core0.cache.l2.prefetches.dropped", 0, &context);
        let started = rec.read(&ctx.sim, "core0.cache.l2.prefetches.issued");
        assert!((1..=sent).contains(&started), "{context}: {started} of {sent} started");
        let dropped = rec.read(&starved.sim, "core0.cache.l2.prefetches.dropped");
        let starved_sent = rec.read(&starved.sim, "core0.cache.l1d.prefetches.store_stream");
        assert_eq!(dropped, starved_sent, "{context}: one MSHR is never given to a prefetch");
    }
}

/// `hierarchy` with every prefetcher on, the load/store unit's included,
/// and the L2 and LLC inclusive of the levels above them.
fn everything_on(backend: BackendKind) -> Config {
    let mut config = prefetching_hierarchy(backend);
    config.cache.inclusion_policy = InclusionPolicy::Inclusive;
    config.cache.load_prefetcher = crate::config::LoadPrefetcherConfig::Stride {
        table_size: 64,
        l1_lines: 2,
        l2_lines: 4,
        page_boundary: crate::config::PageBoundary::Stop,
    };
    config
}

fn stats_a_level_cannot_count_stay_zero(rec: &mut Recorder) {
    let never = [
        // An instruction cache never holds a dirty line.
        ("core0.cache.l1i.writebacks", "nothing to write back"),
        // Only the L1D has a store-miss prefetcher.
        ("core0.cache.l1i.prefetches.store_stream", "no store prefetcher"),
        ("core0.cache.l2.prefetches.store_stream", "no store prefetcher"),
        ("llc.prefetches.store_stream", "no store prefetcher"),
        // Prefetch requests come from above: none reach the L1I, and none
        // ask for the LLC.
        ("core0.cache.l1i.prefetches.dropped", "no prefetch request reaches it"),
        ("llc.prefetches.dropped", "no prefetch request targets it"),
        // Fetch keeps one line request in flight, and a prefetch never
        // takes the last MSHR.
        ("core0.cache.l1i.blocked_requests", "one fetch in flight"),
        // Nothing below the LLC evicts its lines.
        ("llc.back_invalidations", "no level below it"),
    ];
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&everything_on(backend), read_modify_write_lines(512), &context);

        for (path, why) in never {
            rec.expect(&ctx.sim, path, 0, &format!("{context}: {why}"));
        }
    }
}

fn every_level_writes_back_and_drops_what_an_inclusive_level_below_evicts(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&everything_on(backend), read_modify_write_lines(512), &context);

        for cache in ["core0.cache.l1d", "core0.cache.l2"] {
            let written = rec.read(&ctx.sim, &format!("{cache}.writebacks"));
            let evicted = rec.read(&ctx.sim, &format!("{cache}.evictions"));
            assert!((1..=evicted).contains(&written), "{context}: {cache}: {written} of {evicted}");
        }
        // Each level's own evictions run ahead of the bigger level's below
        // it, which finds nothing left to drop; the L1I's code lines are
        // still held when the L2 evicts them.
        let dropped = rec.read(&ctx.sim, "core0.cache.l1i.back_invalidations");
        assert!(dropped > 0, "{context}: the code lines evicted from the L2");
    }
}

fn misses_beyond_each_levels_mshrs_wait_there(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let program =
        ending_in_spin((0..8).map(|n| i().ld(T0 + n % 3, A1, 64 * n as i32).build()).collect());
    let context = "OutOfOrder width 4";
    let mut config = hierarchy(BackendKind::OutOfOrder);
    config.pipeline.width = 4;
    config.cache.l1_d.mshr_count = count(8);
    config.cache.l2.mshr_count = count(2);
    config.cache.l3.mshr_count = count(1);

    let ctx = run_settled(&config, program, context);

    for cache in ["core0.cache.l2", "llc"] {
        let blocked = rec.read(&ctx.sim, &format!("{cache}.blocked_requests"));
        assert!(blocked > 0, "{context}: {cache}: more misses than MSHRs");
    }
}

fn next_line_candidates_past_the_page_are_dropped_at_every_level(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let page_end = super::program::PROGRAM_BASE + 4096 - 4;
    // Loads the data page's last line, then jumps to a spin in the last
    // word of the code page.
    let program = vec![
        i().lui(T1, 1).build(),
        i().add(T1, A1, T1).build(),
        i().ld(T2, T1, -64).build(),
        i().add(T2, T2, T2).build(),
        i().lui(T0, 1).build(),
        i().auipc(T1, 0).build(),
        i().add(T1, T1, T0).build(),
        i().jalr(0, T1, -4 - 20).build(),
    ];
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = hierarchy(backend);
        for cache in [&mut config.cache.l1_i, &mut config.cache.l2, &mut config.cache.l3] {
            cache.prefetcher = PrefetcherKind::NextLine;
        }
        let mut ctx = system_with(&config, &program, &[]);
        super::program::store_words(&mut ctx, page_end, &[i().jal(0, 0).build()]);

        run_to_pc(&mut ctx, page_end, &context);
        ctx.run(SETTLE);

        for cache in ["core0.cache.l1i", "core0.cache.l2", "llc"] {
            let crossing = rec.read(&ctx.sim, &format!("{cache}.prefetches.page_crossing"));
            assert!(crossing > 0, "{context}: {cache}");
        }
    }
}

fn load_prefetches_the_l1d_cannot_take_are_dropped(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut dropped = [0; 2];
        for (n, mshrs) in [2, 16].into_iter().enumerate() {
            let mut config = everything_on(backend);
            config.cache.l1_d.prefetcher = PrefetcherKind::None;
            config.cache.l1_d.mshr_count = count(mshrs);

            let ctx = run_settled(&config, read_modify_write_lines(48), &context);

            dropped[n] = rec.read(&ctx.sim, "core0.cache.l1d.prefetches.dropped");
        }
        assert!(dropped[0] > dropped[1], "{context}: two MSHRs against sixteen: {dropped:?}");
    }
}

/// Where the rewritten code lies: a line of its own, past the program.
const CODE_OFFSET: i32 = 0x200;

/// Copies the instruction at `CODE_OFFSET` over itself through the L1D,
/// which then holds that code line dirty, runs FENCE.I and jumps there;
/// that instruction jumps back to the spin.
fn rewrite_code_then_run_it() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(T0, 0).build(),
        i().lw(T1, T0, CODE_OFFSET).build(),
        i().sw(T0, T1, CODE_OFFSET).build(),
        FENCE_I,
        i().jalr(0, T0, CODE_OFFSET).build(),
    ];
    let spin = 4 * program.len() as i32;
    program.push(i().jal(0, 0).build());
    program.resize(CODE_OFFSET as usize / 4, i().nop().build());
    program.push(i().jal(0, spin - CODE_OFFSET).build());
    (program, PROGRAM_BASE + spin as u64)
}

fn an_instruction_fetch_probes_the_l1d_through_the_l2(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");

        let ctx = run_settled(&hierarchy(backend), rewrite_code_then_run_it(), &context);

        rec.expect(&ctx.sim, "core0.cache.l2.fetch_probes", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l2.fetch_probes_dirty", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.probes", 1, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.writebacks", 1, &context);
        for cache in CACHES {
            rec.expect(&ctx.sim, &format!("{cache}.flushes"), 0, &context);
            rec.expect(&ctx.sim, &format!("{cache}.flushed_lines"), 0, &context);
        }
        for cache in ["core0.cache.l1i", "core0.cache.l1d", "llc"] {
            for stat in ["fetch_probes", "fetch_probes_dirty"] {
                let why = format!("{context}: only where the L1I and L1D meet");
                rec.expect(&ctx.sim, &format!("{cache}.{stat}"), 0, &why);
            }
        }
    }
}

/// Writes three lines, then runs FENCE.I twice.
fn three_dirty_lines_then_two_fence_is() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().addi(T0, 0, 1).build(),
        i().sd(A1, T0, 0).build(),
        i().sd(A1, T0, 64).build(),
        i().sd(A1, T0, 128).build(),
        FENCE_I,
        FENCE_I,
    ])
}

fn fence_i_flushes_the_l1d_when_no_l2_joins_the_l1s(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = hierarchy(backend);
        config.cache.l2.enabled = false;

        let ctx = run_settled(&config, three_dirty_lines_then_two_fence_is(), &context);

        let why = format!("{context}: the second FENCE.I finds nothing fetched since the first");
        rec.expect(&ctx.sim, "core0.cache.l1d.flushes", 1, &why);
        rec.expect(&ctx.sim, "core0.cache.l1d.flushed_lines", 3, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.writebacks", 3, &context);
        rec.expect(&ctx.sim, "core0.cache.l1d.evictions", 0, &context);
        assert_eq!(held_lines(&ctx, "core0.cache.l1d"), 0, "{context}: the flush invalidates");
        for cache in ["core0.cache.l1i", "core0.cache.l2", "llc"] {
            rec.expect(&ctx.sim, &format!("{cache}.flushes"), 0, &context);
            rec.expect(&ctx.sim, &format!("{cache}.flushed_lines"), 0, &context);
        }
    }
}

accounting_checks!(
    every_level_fills_each_fetch_once_and_holds_what_it_did_not_evict,
    a_cold_line_misses_once_and_then_hits,
    a_dirty_victim_is_evicted_and_written_back,
    an_inclusive_l2_eviction_drops_the_l1_copy,
    a_back_invalidation_for_a_line_already_gone_drops_nothing,
    an_inclusive_llc_eviction_drops_the_l2_copy,
    a_cbo_passes_through_and_counts_at_every_level,
    misses_beyond_the_mshrs_wait_as_blocked_requests,
    a_next_line_candidate_past_the_page_is_dropped,
    store_prefetches_go_to_the_l2_and_are_dropped_without_an_mshr,
    stats_a_level_cannot_count_stay_zero,
    every_level_writes_back_and_drops_what_an_inclusive_level_below_evicts,
    misses_beyond_each_levels_mshrs_wait_there,
    next_line_candidates_past_the_page_are_dropped_at_every_level,
    load_prefetches_the_l1d_cannot_take_are_dropped,
    an_instruction_fetch_probes_the_l1d_through_the_l2,
    fence_i_flushes_the_l1d_when_no_l2_joins_the_l1s,
);

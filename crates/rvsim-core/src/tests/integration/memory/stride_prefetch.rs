//! The L1D's stride prefetchers, in the cache and in the load/store unit,
//! learn a load's stride from its PC.
//!
//! A loop whose load strides several lines per iteration lands every
//! access on a different line, so only a prefetcher that tracks the load
//! itself can learn the stride.

use crate::config::Console;
use crate::config::{BackendKind, Config, LoadPrefetcherConfig, PageBoundary, PrefetcherKind};
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, T0, T1, T2, X0};
const DONE_REG: RegIdx = RegIdx::new(2);
const DONE: u64 = 7;
const ITERATIONS: i32 = 64;
const STRIDE: i32 = 256;

/// `ITERATIONS` loads `STRIDE` bytes apart from one load instruction, each
/// address depending on the previous load's (zero) value so the misses
/// cannot overlap.
fn strided_loop() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T0, 0).build(),
        i().lui(A0, 0x10).build(),
        i().add(A0, A0, T0).build(),
        i().addi(T1, X0, ITERATIONS).build(),
        i().ld(T2, A0, 0).build(),
        i().add(A0, A0, T2).build(),
        i().addi(A0, A0, STRIDE).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, X0, -16).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]
}

/// The L1D prefetcher a run uses.
#[derive(Clone, Copy, Debug)]
enum L1dPrefetcher {
    None,
    /// The cache-side stride table.
    InTheCache,
    /// The load/store unit's prefetcher.
    InTheLoadStoreUnit(PageBoundary),
}

/// The machine a run uses, with `prefetcher` on the L1D.
fn machine(backend: BackendKind, prefetcher: L1dPrefetcher) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.prefetcher = PrefetcherKind::Off;
    match prefetcher {
        L1dPrefetcher::None => {}
        L1dPrefetcher::InTheCache => {
            config.cache.l1_d.prefetcher = PrefetcherKind::Stride;
            config.cache.l1_d.prefetch_degree = std::num::NonZero::new(4).unwrap();
            config.cache.l1_d.prefetch_table_size = 64;
        }
        L1dPrefetcher::InTheLoadStoreUnit(page_boundary) => {
            config.cache.load_prefetcher = LoadPrefetcherConfig::Stride {
                table_size: 64,
                l1_lines: 4,
                l2_lines: 0,
                page_boundary,
            };
        }
    }
    // The harness gives memory one cycle; a slow L2 makes a miss cost.
    config.cache.l2.enabled = true;
    config.cache.l2.latency = 60;
    config.system.console = Console::Quiet;
    config
}

/// Runs the loop to its end on `config`, returning the machine and the
/// cycles taken.
fn run_loop(config: &Config) -> (TestContext, u64) {
    let mut ctx = TestContext::with_fast_memory(config).load_program(PROGRAM_BASE, &strided_loop());
    let cycles = ctx.run_until(50_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("loop finished");
    (ctx, cycles)
}

/// Cycles the loop takes.
fn cycles(backend: BackendKind, prefetcher: L1dPrefetcher) -> u64 {
    run_loop(&machine(backend, prefetcher)).1
}

/// The dependent misses serialise without a prefetcher; with one that has
/// learned the stride, most loads find their line already on its way.
fn assert_the_stride_is_learned(backend: BackendKind, prefetcher: L1dPrefetcher) {
    let cycles_without = cycles(backend, L1dPrefetcher::None);
    let cycles_with = cycles(backend, prefetcher);

    assert!(
        cycles_with * 2 < cycles_without,
        "{backend:?} {prefetcher:?}: {cycles_with} cycles with, {cycles_without} without"
    );
}

#[test]
fn inorder_a_multi_line_stride_is_prefetched_by_the_cache() {
    assert_the_stride_is_learned(BackendKind::InOrder, L1dPrefetcher::InTheCache);
}

#[test]
fn o3_a_multi_line_stride_is_prefetched_by_the_cache() {
    assert_the_stride_is_learned(BackendKind::OutOfOrder, L1dPrefetcher::InTheCache);
}

#[test]
fn inorder_a_multi_line_stride_is_prefetched_by_the_load_store_unit() {
    for page_boundary in [PageBoundary::Stop, PageBoundary::CrossWithTlb] {
        let prefetcher = L1dPrefetcher::InTheLoadStoreUnit(page_boundary);
        assert_the_stride_is_learned(BackendKind::InOrder, prefetcher);
    }
}

#[test]
fn o3_a_multi_line_stride_is_prefetched_by_the_load_store_unit() {
    for page_boundary in [PageBoundary::Stop, PageBoundary::CrossWithTlb] {
        let prefetcher = L1dPrefetcher::InTheLoadStoreUnit(page_boundary);
        assert_the_stride_is_learned(BackendKind::OutOfOrder, prefetcher);
    }
}

/// Without translation a page is the smallest one, so `Stop` leaves the
/// first lines of each new 4 KiB page to demand misses that crossing
/// prefetches.
#[test]
fn crossing_pages_beats_stopping_at_them_on_a_stream_through_several() {
    let stop =
        cycles(BackendKind::OutOfOrder, L1dPrefetcher::InTheLoadStoreUnit(PageBoundary::Stop));
    let cross = cycles(
        BackendKind::OutOfOrder,
        L1dPrefetcher::InTheLoadStoreUnit(PageBoundary::CrossWithTlb),
    );

    assert!(cross < stop, "crossing {cross} cycles, stopping {stop}");
}

/// The prefetches the load/store unit's prefetcher sent into the L1D and
/// into the L2, for a loop run with `l1_lines` and `l2_lines`.
fn load_prefetches_sent(l1_lines: usize, l2_lines: usize) -> (f64, f64) {
    let mut config =
        machine(BackendKind::InOrder, L1dPrefetcher::InTheLoadStoreUnit(PageBoundary::Stop));
    config.cache.load_prefetcher = LoadPrefetcherConfig::Stride {
        table_size: 64,
        l1_lines,
        l2_lines,
        page_boundary: PageBoundary::Stop,
    };
    let (ctx, _) = run_loop(&config);
    let paths = ctx.sim.state.cores[0].units.stat_paths.load_prefetch;
    let stats = &ctx.sim.state.uncore.kernel.stats;
    (stats.get(paths.l1).unwrap_or(0.0), stats.get(paths.l2).unwrap_or(0.0))
}

#[test]
fn prefetches_into_the_l1d_count_under_the_l1_stat_alone() {
    let (l1, l2) = load_prefetches_sent(4, 0);

    assert!(l1 > 0.0, "{l1} L1D prefetches");
    assert_eq!(l2, 0.0);
}

#[test]
fn prefetches_further_ahead_count_under_the_l2_stat() {
    let (l1, l2) = load_prefetches_sent(4, 8);

    assert!(l1 > 0.0, "{l1} L1D prefetches");
    assert!(l2 > 0.0, "{l2} L2 prefetches");
}

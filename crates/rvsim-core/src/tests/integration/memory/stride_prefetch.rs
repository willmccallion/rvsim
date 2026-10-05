//! The L1D's stride prefetchers, in the cache and in the load/store unit,
//! learn a load's stride from its PC.
//!
//! A loop whose load strides several lines per iteration lands every
//! access on a different line, so only a prefetcher that tracks the load
//! itself can learn the stride.

use crate::config::{BackendKind, Config, LoadPrefetcherConfig, PageBoundary, PrefetcherKind};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const A0: u32 = 10;
const DONE_REG: u32 = 2;
const DONE: u64 = 7;
const PROGRAM_BASE: u64 = 0x8000_0000;
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
        i().addi(T1, 0, ITERATIONS).build(),
        i().ld(T2, A0, 0).build(),
        i().add(A0, A0, T2).build(),
        i().addi(A0, A0, STRIDE).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -16).build(),
        i().addi(DONE_REG, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
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

/// Cycles the loop takes.
fn cycles(backend: BackendKind, prefetcher: L1dPrefetcher) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.prefetcher = PrefetcherKind::None;
    match prefetcher {
        L1dPrefetcher::None => {}
        L1dPrefetcher::InTheCache => {
            config.cache.l1_d.prefetcher = PrefetcherKind::Stride;
            config.cache.l1_d.prefetch_degree = 4;
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
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &strided_loop());

    ctx.run_until(50_000, |ctx| ctx.get_reg(DONE_REG as usize) == DONE).expect("loop finished")
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

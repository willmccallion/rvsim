//! FENCE.I makes the hart's stores visible to its instruction fetches.
//! With no cache below joining the L1I and L1D, it flushes the L1D: a walk
//! over every line that writes back the dirty ones, skipped when nothing
//! has been fetched since the last flush. With an L2, the L2 probes the
//! L1D on an instruction fetch instead, and FENCE.I costs the same however
//! much the L1D holds dirty.

use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::{FENCE_I, FENCE_IORW, InstructionBuilder};
use crate::tests::support::count;
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const A1: u32 = 11;
const S2: u32 = 18;
const S3: u32 = 19;
const S4: u32 = 20;
const PROGRAM_BASE: u64 = 0x8000_0000;
const MCYCLE: u32 = 0xB00;
/// Lines in the 1 KiB direct-mapped L1D.
const L1D_LINES: u64 = 16;

/// What the L1D holds when FENCE.I runs.
#[derive(Clone, Copy, Debug)]
enum L1d {
    /// Nothing fetched.
    Untouched,
    /// One line loaded, clean.
    OneCleanLine,
    /// This many lines written.
    Dirty(i32),
}

/// Fills the L1D as `l1d` says, drains its writes, then times one FENCE.I
/// with `mcycle` into `s3 - s2`.
fn program(l1d: L1d) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().auipc(A1, 2).build(), i().addi(T0, 0, 1).build()];
    match l1d {
        L1d::Untouched => {}
        L1d::OneCleanLine => program.push(i().ld(T1, A1, 0).build()),
        L1d::Dirty(lines) => {
            for line in 0..lines {
                program.push(i().sd(A1, T0, 64 * line).build());
            }
        }
    }
    program.extend([
        FENCE_IORW,
        i().csrrs(S2, MCYCLE, 0).build(),
        FENCE_I,
        i().csrrs(S3, MCYCLE, 0).build(),
        i().addi(S4, 0, 1).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

fn config(backend: BackendKind, with_l2: bool) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let l1_d = &mut config.cache.l1_d;
    l1_d.enabled = true;
    l1_d.size_bytes = 1024;
    l1_d.ways = 1;
    l1_d.mshr_count = count(1);
    l1_d.write_buffers = count(1);
    config.cache.l2.enabled = with_l2;
    config
}

/// Cycles one FENCE.I took with the L1D as `l1d` says.
fn fence_i_cycles(backend: BackendKind, with_l2: bool, l1d: L1d) -> u64 {
    let config = config(backend, with_l2);
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program(l1d));

    let finished = ctx.run_until(20_000, |ctx| ctx.get_reg(S4 as usize) == 1);

    assert!(finished.is_some(), "{backend:?}, L2 {with_l2}, {l1d:?}: did not finish");
    ctx.get_reg(S3 as usize) - ctx.get_reg(S2 as usize)
}

/// The timed window includes refetching the instruction after the FENCE.I
/// through the invalidated L1I, which costs the same for every L1D state
/// of one configuration, so each cost is measured against the untouched
/// L1D of the same configuration.
#[test]
fn without_an_l2_fence_i_walks_the_l1d_and_writes_back_each_dirty_line() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let skipped = fence_i_cycles(backend, false, L1d::Untouched);
        let clean = fence_i_cycles(backend, false, L1d::OneCleanLine);
        let dirty = fence_i_cycles(backend, false, L1d::Dirty(8));

        assert!(
            clean >= skipped + L1D_LINES,
            "{backend:?}: the walk took {} cycles",
            clean - skipped
        );
        assert!(dirty >= clean + 8, "{backend:?}: 8 writebacks took {} cycles", dirty - clean);
    }
}

#[test]
fn without_an_l2_fence_i_skips_the_walk_when_nothing_was_fetched() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let walked = fence_i_cycles(backend, false, L1d::OneCleanLine);

        let skipped = fence_i_cycles(backend, false, L1d::Untouched);

        assert!(
            skipped + L1D_LINES <= walked,
            "{backend:?}: {skipped} cycles with nothing fetched, {walked} with a line to walk"
        );
    }
}

#[test]
fn with_an_l2_fence_i_costs_the_same_however_many_lines_are_dirty() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let clean = fence_i_cycles(backend, true, L1d::OneCleanLine);
        let dirty = fence_i_cycles(backend, true, L1d::Dirty(8));

        assert_eq!(dirty, clean, "{backend:?}");
    }
}

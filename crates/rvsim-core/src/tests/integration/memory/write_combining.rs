//! A write-combining buffer holds committed stores until it writes their line.
//!
//! The hart's own loads read them from it, a load it holds only part of
//! waits for the line to be written, every store reaches memory without a
//! fence once the write port is idle, and a sent line holds its entry until
//! the L1D has written it, so a slow L1D holds back the stores behind it.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::X0;
use std::num::NonZeroUsize;

const DATA: u64 = PROGRAM_BASE + 0x400;
const OTHER_LINE: i32 = 0x80;
const BASE: RegIdx = RegIdx::new(5);
const LOW: RegIdx = RegIdx::new(6);
const HIGH: RegIdx = RegIdx::new(7);
const WHOLE: RegIdx = RegIdx::new(10);
const UNWRITTEN: RegIdx = RegIdx::new(11);
const PARTLY_WRITTEN: RegIdx = RegIdx::new(12);
const AFTER_EVICTION: RegIdx = RegIdx::new(13);
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: u64 = 7;
const LOW_VALUE: u64 = 0x123;
const HIGH_VALUE: u64 = 0x456;

fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(BASE, 0).build(),
        i().addi(BASE, BASE, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(LOW, X0, LOW_VALUE as i32).build(),
        i().addi(HIGH, X0, HIGH_VALUE as i32).build(),
        i().sw(BASE, LOW, 0).build(),
        i().sw(BASE, HIGH, 4).build(),
        i().ld(WHOLE, BASE, 0).build(),
        i().lw(UNWRITTEN, BASE, 8).build(),
        i().sw(BASE, LOW, 12).build(),
        i().ld(PARTLY_WRITTEN, BASE, 8).build(),
        i().sw(BASE, HIGH, OTHER_LINE).build(),
        i().ld(AFTER_EVICTION, BASE, 0).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]
}

fn run(backend: BackendKind, wcb_entries: usize) -> TestContext {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.cache.wcb_entries = wcb_entries;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());
    ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx
}

#[test]
fn loads_see_the_stores_the_buffer_holds() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        for entries in [2, 4] {
            let ctx = run(backend, entries);

            let context = format!("{backend:?} with {entries} entries");
            assert_eq!(ctx.get_reg(WHOLE), HIGH_VALUE << 32 | LOW_VALUE, "{context}");
            assert_eq!(ctx.get_reg(UNWRITTEN), 0, "{context}");
            assert_eq!(ctx.get_reg(PARTLY_WRITTEN), LOW_VALUE << 32, "{context}");
            assert_eq!(ctx.get_reg(AFTER_EVICTION), HIGH_VALUE << 32 | LOW_VALUE, "{context}");
        }
    }
}

#[test]
fn every_store_reaches_memory_without_a_fence() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = run(backend, 4);

        ctx.run(200);

        let word = |ctx: &mut TestContext, offset: u64| {
            ctx.sim.probe_mem_load(PhysAddr::new(DATA + offset), ProbeWidth::B4)
        };
        assert_eq!(word(&mut ctx, 0), LOW_VALUE, "{backend:?}");
        assert_eq!(word(&mut ctx, 4), HIGH_VALUE, "{backend:?}");
        assert_eq!(word(&mut ctx, 12), LOW_VALUE, "{backend:?}");
        assert_eq!(word(&mut ctx, OTHER_LINE as u64), HIGH_VALUE, "{backend:?}");
    }
}

const STREAM_COUNT: RegIdx = RegIdx::new(8);
/// Lines the stream writes, a doubleword at a time.
const STREAM_LINES: i32 = 128;
const LINE_BYTES: i32 = 64;
const STREAM_CYCLES: u64 = 100_000;
/// Slower to write a line than the stream is to fill one.
const SLOW_L1D_LATENCY: u64 = 50;

/// Writes `STREAM_LINES` consecutive lines from `DATA`, a line a loop.
fn store_stream() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(BASE, 0).build(),
        i().addi(BASE, BASE, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(STREAM_COUNT, X0, STREAM_LINES).build(),
    ];
    let stores: Vec<u32> =
        (0..LINE_BYTES).step_by(8).map(|offset| i().sd(BASE, BASE, offset).build()).collect();
    let loop_bytes = 4 * (stores.len() as i32 + 2);
    program.extend(stores);
    program.extend([
        i().addi(BASE, BASE, LINE_BYTES).build(),
        i().addi(STREAM_COUNT, STREAM_COUNT, -1).build(),
        i().bne(STREAM_COUNT, X0, -loop_bytes).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]);
    program
}

#[test]
fn a_slow_l1d_holds_back_the_stores_behind_the_buffer() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut config = Config::minimal();
        config.pipeline.backend = backend;
        config.cache.l1_d.enabled = true;
        config.cache.l1_d.mshr_count = NonZeroUsize::MIN;
        config.cache.l1_d.latency = SLOW_L1D_LATENCY;
        config.cache.wcb_entries = 4;
        let held = config.cache.wcb_entries + config.pipeline.store_buffer_size;
        let mut ctx =
            TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &store_stream());

        let mut most_waiting = 0;
        for _ in 0..STREAM_CYCLES {
            let core = &ctx.sim.state.cores[0];
            most_waiting = most_waiting.max(core.units.l1_d_cache.blocked_requests());
            if ctx.get_reg(DONE_REG) == DONE && !core.pipeline.has_committed_writes(&core.units) {
                break;
            }
            ctx.run(1);
        }

        assert_eq!(ctx.get_reg(DONE_REG), DONE, "{backend:?} finished the stream");
        assert!(most_waiting <= held, "{backend:?}: {most_waiting} writes waited for the L1D");
    }
}

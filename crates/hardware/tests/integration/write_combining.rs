//! A write-combining buffer holds committed stores until it writes their line.
//!
//! The hart's own loads read them from it, a load it holds only part of
//! waits for the line to be written, and every store reaches memory without
//! a fence once the write port is idle.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::uarch::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const OTHER_LINE: i32 = 0x80;
const BASE: u32 = 5;
const LOW: u32 = 6;
const HIGH: u32 = 7;
const WHOLE: usize = 10;
const UNWRITTEN: usize = 11;
const PARTLY_WRITTEN: usize = 12;
const AFTER_EVICTION: usize = 13;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const LOW_VALUE: u64 = 0x123;
const HIGH_VALUE: u64 = 0x456;

fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(BASE, 0).build(),
        i().addi(BASE, BASE, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(LOW, 0, LOW_VALUE as i32).build(),
        i().addi(HIGH, 0, HIGH_VALUE as i32).build(),
        i().sw(BASE, LOW, 0).build(),
        i().sw(BASE, HIGH, 4).build(),
        i().ld(WHOLE as u32, BASE, 0).build(),
        i().lw(UNWRITTEN as u32, BASE, 8).build(),
        i().sw(BASE, LOW, 12).build(),
        i().ld(PARTLY_WRITTEN as u32, BASE, 8).build(),
        i().sw(BASE, HIGH, OTHER_LINE).build(),
        i().ld(AFTER_EVICTION as u32, BASE, 0).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

fn run(backend: BackendType, wcb_entries: usize) -> TestContext {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.cache.wcb_entries = wcb_entries;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx
}

#[test]
fn loads_see_the_stores_the_buffer_holds() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        for entries in [1, 4] {
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
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run(backend, 4);

        ctx.run(200);

        let word = |ctx: &mut TestContext, offset: u64| {
            ctx.sim.probe_mem_load(PhysAddr::new(DATA + offset), 4)
        };
        assert_eq!(word(&mut ctx, 0), LOW_VALUE, "{backend:?}");
        assert_eq!(word(&mut ctx, 4), HIGH_VALUE, "{backend:?}");
        assert_eq!(word(&mut ctx, 12), LOW_VALUE, "{backend:?}");
        assert_eq!(word(&mut ctx, OTHER_LINE as u64), HIGH_VALUE, "{backend:?}");
    }
}

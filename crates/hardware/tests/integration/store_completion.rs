//! Stores complete when the cache takes their write.
//!
//! A committed store keeps its store-buffer slot until the cache has taken
//! its write, and a fence retires only once older stores have: stores that
//! miss hold the buffer, and a fence behind a missing store waits for it.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA_OFFSET: i32 = 0x400;
const LINE: i32 = 64;
const STORES: i32 = 8;
const DONE_REG: usize = 2;
const DONE: u64 = 7;
/// `fence rw, rw`.
const FENCE_RW_RW: u32 = 0x0330_000F;

fn config(backend: BackendType, store_buffer_size: usize) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.store_buffer_size = store_buffer_size;
    config.cache.l1_i.enabled = true;
    config.cache.l1_d.enabled = true;
    config
}

fn cycles_to_finish(config: &Config, program: &[u32]) -> u64 {
    let mut ctx = TestContext::new_with_config(config).load_program(PROGRAM_BASE, program);
    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished")
}

fn done() -> u32 {
    InstructionBuilder::new().addi(DONE_REG as u32, 0, DONE as i32).build()
}

/// `STORES` stores `stride` bytes apart, then the completion marker.
fn stores(stride: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().auipc(10, 0).build(), i().addi(10, 10, DATA_OFFSET).build()];
    program.extend((0..STORES).map(|n| i().sd(10, 0, n * stride).build()));
    program.push(done());
    program.push(i().jal(0, 0).build());
    program
}

/// A store to a line not yet cached, then `between`, then the marker.
fn store_then(between: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, DATA_OFFSET).build(),
        i().sd(10, 0, 0).build(),
        between,
        done(),
        i().jal(0, 0).build(),
    ]
}

#[test]
fn a_store_that_misses_holds_its_slot_until_the_line_arrives() {
    const MISS_COST: u64 = 5;
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let missing = cycles_to_finish(&config(backend, 1), &stores(LINE));
        let hitting = cycles_to_finish(&config(backend, 1), &stores(8));

        assert!(
            missing > hitting + (STORES as u64 - 1) * MISS_COST,
            "{backend:?}: stores to distinct lines {missing} cycles vs one line {hitting}"
        );
    }
}

#[test]
fn a_fence_waits_for_an_older_store_that_misses() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let nop = InstructionBuilder::new().nop().build();
        let without_fence = cycles_to_finish(&config(backend, 16), &store_then(nop));
        let with_fence = cycles_to_finish(&config(backend, 16), &store_then(FENCE_RW_RW));

        assert!(
            with_fence > without_fence + 5,
            "{backend:?}: fenced {with_fence} cycles vs unfenced {without_fence}"
        );
    }
}

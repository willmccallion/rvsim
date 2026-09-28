//! A store-conditional executes non-speculatively at the head of the ROB
//! and the cache decides its result as it performs it, so a failing one
//! returns 1 without the pipeline refetching what follows it.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const SC_RESULT: usize = 7;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const PLAIN_STORE_VALUE: u64 = 0x11;
const SC_VALUE: u64 = 0x22;

/// `load` (an `lr.d`, or a plain load that takes no reservation), then
/// `between`, then an `sc.d` of `SC_VALUE`, then the marker.
fn program(load: u32, between: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, 0x400).build(),
        i().addi(11, 0, PLAIN_STORE_VALUE as i32).build(),
        i().addi(12, 0, SC_VALUE as i32).build(),
        load,
        between,
        i().sc_d(SC_RESULT as u32, 10, 12).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

struct Outcome {
    sc_result: u64,
    memory: u64,
    cycles: u64,
}

fn run(backend: BackendType, load: u32, between: u32) -> Outcome {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program(load, between));
    let cycles =
        ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    Outcome {
        sc_result: ctx.get_reg(SC_RESULT),
        memory: ctx.sim.probe_mem_load(PhysAddr::new(DATA), 8),
        cycles,
    }
}

fn lr() -> u32 {
    InstructionBuilder::new().lr_d(5, 10).build()
}

fn nop() -> u32 {
    InstructionBuilder::new().nop().build()
}

#[test]
fn a_store_conditional_with_its_reservation_writes_and_returns_zero() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let outcome = run(backend, lr(), nop());

        assert_eq!((outcome.sc_result, outcome.memory), (0, SC_VALUE), "{backend:?}");
    }
}

#[test]
fn a_store_conditional_after_a_store_to_its_reservation_fails() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let outcome = run(backend, lr(), InstructionBuilder::new().sd(10, 11, 0).build());

        assert_eq!((outcome.sc_result, outcome.memory), (1, PLAIN_STORE_VALUE), "{backend:?}");
    }
}

#[test]
fn a_failing_store_conditional_costs_no_more_than_a_succeeding_one() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let succeeding = run(backend, lr(), nop());
        let failing = run(backend, InstructionBuilder::new().ld(5, 10, 0).build(), nop());

        assert_eq!(failing.sc_result, 1, "{backend:?}: no reservation, so it fails");
        assert!(
            failing.cycles <= succeeding.cycles,
            "{backend:?}: failing {} cycles vs succeeding {}: no refetch after a failure",
            failing.cycles,
            succeeding.cycles
        );
    }
}

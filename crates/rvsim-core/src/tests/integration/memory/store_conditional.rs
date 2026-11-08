//! A store-conditional executes non-speculatively at the head of the ROB
//! and the cache decides its result as it performs it, so a failing one
//! returns 1 without the pipeline refetching what follows it.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::{X0, X5, X10, X11, X12};

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const SC_RESULT: RegIdx = RegIdx::new(7);
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: u64 = 7;
const PLAIN_STORE_VALUE: u64 = 0x11;
const SC_VALUE: u64 = 0x22;

/// `load` (an `lr.d`, or a plain load that takes no reservation), then
/// `between`, then an `sc.d` of `SC_VALUE`, then the marker.
fn program(load: u32, between: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(X10, 0).build(),
        i().addi(X10, X10, 0x400).build(),
        i().addi(X11, X0, PLAIN_STORE_VALUE as i32).build(),
        i().addi(X12, X0, SC_VALUE as i32).build(),
        load,
        between,
        i().sc_d(SC_RESULT, X10, X12).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]
}

struct Outcome {
    sc_result: u64,
    memory: u64,
    cycles: u64,
}

fn run(backend: BackendKind, load: u32, between: u32) -> Outcome {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let mut ctx =
        TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program(load, between));
    let cycles =
        ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    Outcome {
        sc_result: ctx.get_reg(SC_RESULT),
        memory: ctx.sim.probe_mem_load(PhysAddr::new(DATA), ProbeWidth::B8),
        cycles,
    }
}

fn lr() -> u32 {
    InstructionBuilder::new().lr_d(X5, X10).build()
}

fn nop() -> u32 {
    InstructionBuilder::new().nop().build()
}

#[test]
fn a_store_conditional_with_its_reservation_writes_and_returns_zero() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let outcome = run(backend, lr(), nop());

        assert_eq!((outcome.sc_result, outcome.memory), (0, SC_VALUE), "{backend:?}");
    }
}

#[test]
fn a_store_conditional_after_a_store_to_its_reservation_fails() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let outcome = run(backend, lr(), InstructionBuilder::new().sd(X10, X11, 0).build());

        assert_eq!((outcome.sc_result, outcome.memory), (1, PLAIN_STORE_VALUE), "{backend:?}");
    }
}

#[test]
fn a_failing_store_conditional_costs_no_more_than_a_succeeding_one() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let succeeding = run(backend, lr(), nop());
        let failing = run(backend, InstructionBuilder::new().ld(X5, X10, 0).build(), nop());

        assert_eq!(failing.sc_result, 1, "{backend:?}: no reservation, so it fails");
        assert!(
            failing.cycles <= succeeding.cycles,
            "{backend:?}: failing {} cycles vs succeeding {}: no refetch after a failure",
            failing.cycles,
            succeeding.cycles
        );
    }
}

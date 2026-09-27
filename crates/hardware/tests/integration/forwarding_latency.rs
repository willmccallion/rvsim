//! A load answered from the store buffer takes as long as an L1D hit.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::{Config, MemDepPredictor};
use rvsim_core::core::pipeline::engine::BackendType;

const T0: u32 = 5;
const T1: u32 = 6;
const A0: u32 = 10;
const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const PAIRS: u64 = 20;

/// Twenty store/load pairs where each load depends on the store just
/// before it and feeds the next store, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(T0, 0, 1).build(),
    ];
    for _ in 0..PAIRS {
        program.push(i().sd(A0, T0, 0).build());
        program.push(i().ld(T1, A0, 0).build());
        program.push(i().addi(T0, T1, 1).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

fn cycles_to_finish(backend: BackendType, l1d_latency: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 2;
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.latency = l1d_latency;
    // Every load waits for its store, so no load speculates past one and
    // replays: the measurement is the forwarding latency alone.
    config.pipeline.mem_dep_predictor = MemDepPredictor::Blind;
    config.system.uart_quiet = true;
    let program = program();
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    let all_but_spin = program.len() as u64 - 1;
    let mut cycles = 0;
    while ctx.sim.state.harts[0].instructions_retired < all_but_spin && cycles < 100_000 {
        ctx.run(1);
        cycles += 1;
    }
    assert!(cycles < 100_000, "{backend:?}: the chain retired");
    assert_eq!(ctx.get_reg(T0 as usize), PAIRS + 1, "{backend:?}: every load saw its store");
    cycles
}

fn check(backend: BackendType) {
    let fast = cycles_to_finish(backend, 1);
    let slow = cycles_to_finish(backend, 9);

    // Each of the twenty dependent pairs pays the eight extra cycles; the
    // out-of-order core overlaps the first pair's store with the setup.
    assert!(
        slow >= fast + (PAIRS - 1) * 8,
        "{backend:?}: L1D latency 9 cost {} cycles over {fast}",
        slow - fast
    );
}

#[test]
fn a_forwarded_load_takes_the_l1d_latency_inorder() {
    check(BackendType::InOrder);
}

#[test]
fn a_forwarded_load_takes_the_l1d_latency_o3() {
    check(BackendType::OutOfOrder);
}

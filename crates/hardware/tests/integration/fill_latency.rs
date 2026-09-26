//! A line that arrives from below is read out through the cache's own
//! access latency before the requests waiting on it are answered.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const T0: u32 = 5;
const A0: u32 = 10;
const PROGRAM_BASE: u64 = 0x8000_0000;
const LINES: u64 = 16;
const LINE_BYTES: u64 = 64;

/// Sixteen dependent loads from sixteen different lines, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program =
        vec![i().lui(A0, 0x1).build(), i().auipc(T0, 0).build(), i().add(A0, A0, T0).build()];
    for line in 0..LINES {
        program.push(i().ld(T0, A0, (line * LINE_BYTES) as i32).build());
        program.push(i().add(A0, A0, T0).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

/// Cycles until every load has retired; every load misses the L1D.
fn cycles_to_finish(l1d_latency: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::InOrder;
    config.pipeline.width = 1;
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.latency = l1d_latency;
    config.system.uart_quiet = true;
    let program = program();
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    let all_but_spin = program.len() as u64 - 1;
    let mut cycles = 0;
    while ctx.sim.state.harts[0].instructions_retired < all_but_spin && cycles < 100_000 {
        ctx.run(1);
        cycles += 1;
    }
    assert!(cycles < 100_000, "every load retired");
    cycles
}

#[test]
fn a_fill_is_answered_after_the_cache_latency() {
    let fast = cycles_to_finish(1);
    let slow = cycles_to_finish(9);

    // Every one of the sixteen misses pays the extra eight cycles twice:
    // once on the request's tag lookup and once when the line arrives.
    assert!(slow >= fast + LINES * 16, "l1d latency 9 cost {} cycles over {fast}", slow - fast);
}

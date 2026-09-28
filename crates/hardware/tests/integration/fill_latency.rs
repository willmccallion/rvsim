//! A fill is forwarded to its waiting requests as it is written.
//!
//! A miss pays the cache's access latency once, for its tag lookup, and the
//! fill costs only the response latency.

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
fn cycles_to_finish(l1d_latency: u64, l1d_response_latency: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::InOrder;
    config.pipeline.width = 1;
    config.cache.l1_d.enabled = true;
    // Memory must not serialise the fills.
    config.memory.simple_bandwidth_gib_s = 1e6;
    config.cache.l1_d.latency = l1d_latency;
    config.cache.l1_d.response_latency = l1d_response_latency;
    config.system.console = rvsim_core::config::Console::Quiet;
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
fn a_miss_pays_the_access_latency_once_for_its_tag_lookup() {
    let fast = cycles_to_finish(1, 1);
    let slow = cycles_to_finish(9, 1);

    let extra = slow - fast;
    assert!(extra >= LINES * 8, "l1d latency 9 cost {extra} cycles over {fast}");
    assert!(extra < LINES * 16, "the fill is not a second array access: {extra} cycles");
}

#[test]
fn a_fill_is_answered_after_the_response_latency() {
    let fast = cycles_to_finish(1, 1);
    let slow = cycles_to_finish(1, 9);

    // Every miss after the first waits on the load before it.
    let serial_misses = LINES - 1;
    assert!(slow >= fast + serial_misses * 8, "response latency 9 cost {} cycles", slow - fast);
}

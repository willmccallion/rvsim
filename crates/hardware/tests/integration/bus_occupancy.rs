//! The system bus carries one transaction at a time per channel, so
//! back-to-back line fills queue for their transfer time.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const T0: u32 = 5;
const A0: u32 = 10;
const PROGRAM_BASE: u64 = 0x8000_0000;
const LINES: u64 = 16;
const LINE_BYTES: u64 = 64;

/// Sixteen independent loads from sixteen different lines, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program =
        vec![i().lui(A0, 0x1).build(), i().auipc(T0, 0).build(), i().add(A0, A0, T0).build()];
    for line in 0..LINES {
        program.push(i().ld(T0, A0, (line * LINE_BYTES) as i32).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

/// Cycles until every load has retired on a core whose L1D misses on
/// every line, so each load is a line transfer over a bus `bus_width`
/// bytes wide.
fn cycles_to_finish(bus_width: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::OutOfOrder;
    config.pipeline.width = 4;
    config.cache.l1_d.enabled = true;
    // Memory must not serialise the fills.
    config.memory.simple_bandwidth_gib_s = 1e6;
    config.system.bus_width = bus_width;
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
fn line_fills_queue_on_the_bus_for_their_transfer_time() {
    let wide = cycles_to_finish(64);
    let narrow = cycles_to_finish(8);

    // A 64-byte response is one transfer on a 64-byte bus and eight on an
    // 8-byte bus: the sixteen responses serialise on the response channel.
    let extra_transfers = (LINES - 1) * 7;
    assert!(
        narrow >= wide + extra_transfers,
        "narrow bus cost {} cycles over {wide}",
        narrow - wide
    );
}

//! Fetch predicts direct jumps, compressed jumps and returns on its own.
//!
//! A JAL's target comes from its immediate, so it never needs the BTB; a
//! call pushes the return stack the moment it is fetched, so the return
//! fetched right behind it is predicted; and compressed control flow is
//! predicted like its 32-bit form.

use crate::support::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const PROGRAM_BASE: u64 = 0x8000_0000;

/// `jal ra, f` sixteen times, `j .`, then `f: ret` (assembled, `.option norvc`).
const CALLS: [u16; 36] = [
    0x00EF, 0x0440, 0x00EF, 0x0400, 0x00EF, 0x03C0, 0x00EF, 0x0380, 0x00EF, 0x0340, 0x00EF, 0x0300,
    0x00EF, 0x02C0, 0x00EF, 0x0280, 0x00EF, 0x0240, 0x00EF, 0x0200, 0x00EF, 0x01C0, 0x00EF, 0x0180,
    0x00EF, 0x0140, 0x00EF, 0x0100, 0x00EF, 0x00C0, 0x00EF, 0x0080, 0x006F, 0x0000, 0x8067, 0x0000,
];
const CALLS_RETIRED: u64 = 16 * 2 + 1;

/// Eight `c.j` over a `c.nop`, eight `jal ra, g`, `c.j .`, then `g: c.jr ra`.
const COMPRESSED: [u16; 34] = [
    0xA011, 0x0001, 0xA011, 0x0001, 0xA011, 0x0001, 0xA011, 0x0001, 0xA011, 0x0001, 0xA011, 0x0001,
    0xA011, 0x0001, 0xA011, 0x0001, 0x00EF, 0x0220, 0x00EF, 0x01E0, 0x00EF, 0x01A0, 0x00EF, 0x0160,
    0x00EF, 0x0120, 0x00EF, 0x00E0, 0x00EF, 0x00A0, 0x00EF, 0x0060, 0xA001, 0x8082,
];
const COMPRESSED_RETIRED: u64 = 8 + 8 * 2 + 1;

fn mispredicts_running(backend: BackendKind, program: &[u16], retired: u64) -> f64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 2;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config);
    for (i, half) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(PROGRAM_BASE + 2 * i as u64), u64::from(*half), 2);
    }
    ctx.sim.state.harts[0].pc = PROGRAM_BASE;
    ctx.sim.sync_arch_regs();

    ctx.run(600);

    assert!(ctx.sim.state.harts[0].instructions_retired >= retired, "{backend:?}: the program ran");
    let path = ctx.sim.state.cores[0].units.stat_paths.bp.spec_mispredicts;
    ctx.sim.state.stats.get(path).unwrap_or(0.0)
}

#[test]
fn calls_and_returns_are_predicted_from_the_fetched_return_stack() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mispredicts = mispredicts_running(backend, &CALLS, CALLS_RETIRED);
        assert_eq!(
            mispredicts, 0.0,
            "{backend:?}: {mispredicts} mispredicts on back-to-back calls"
        );
    }
}

#[test]
fn compressed_jumps_and_returns_are_predicted() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mispredicts = mispredicts_running(backend, &COMPRESSED, COMPRESSED_RETIRED);
        assert_eq!(
            mispredicts, 0.0,
            "{backend:?}: {mispredicts} mispredicts on compressed control flow"
        );
    }
}

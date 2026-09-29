//! A misaligned access that straddles two cache lines is two L1D accesses.

use crate::support::builder::instruction::InstructionBuilder;
use crate::support::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;

/// `ld a1, offset(a0)` then `sd a2, offset+64(a0)` then a spin.
fn program(offset: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().lui(A2, 0x12345).build(),
        i().ld(A1, A0, offset).build(),
        i().sd(A0, A2, offset + 64).build(),
        i().jal(0, 0).build(),
    ]
}

/// Runs the program and returns the loaded value and the L1D accesses.
fn run(backend: BackendKind, offset: i32) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program(offset));
    ctx.sim.probe_mem_store(PhysAddr::new(DATA + offset as u64), 0x1817_1615_1413_1211, 8);

    ctx.run(300);

    let paths = &ctx.sim.state.cores[0].units.l1_d_cache.stat_paths;
    let stats = &ctx.sim.state.stats;
    let accesses = stats.get(paths.hits).unwrap_or(0.0) + stats.get(paths.misses).unwrap_or(0.0);
    (ctx.get_reg(A1 as usize), accesses as u64)
}

fn check(backend: BackendKind) {
    let (within, accesses_within) = run(backend, 8);
    let (crossing, accesses_crossing) = run(backend, 60);

    assert_eq!(within, 0x1817_1615_1413_1211, "{backend:?}: aligned load value");
    assert_eq!(crossing, 0x1817_1615_1413_1211, "{backend:?}: crossing load value");
    assert_eq!(
        accesses_crossing,
        accesses_within + 2,
        "{backend:?}: the crossing load and the crossing store each cost one more L1D access"
    );
}

#[test]
fn a_line_straddling_access_costs_two_cache_accesses_inorder() {
    check(BackendKind::InOrder);
}

#[test]
fn a_line_straddling_access_costs_two_cache_accesses_o3() {
    check(BackendKind::OutOfOrder);
}

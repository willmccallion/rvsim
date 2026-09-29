//! An AMO is one L1D access: the cache takes the line writable, performs
//! the read-modify-write and leaves the line modified, so commit has no
//! second store to send.

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
const AMOS: u64 = 8;

/// `AMOS` `amoadd.d a1, a2, (a0)` on one word, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(A2, 0, 3).build(),
    ];
    program.extend((0..AMOS).map(|_| i().amoadd_d(A1, A0, A2).build()));
    program.push(i().jal(0, 0).build());
    program
}

fn check_one_access_per_amo(backend: BackendKind) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.cache.wcb_entries = 0;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    ctx.sim.probe_mem_store(PhysAddr::new(DATA), 100, 8);

    ctx.run(400);

    let paths = &ctx.sim.state.cores[0].units.l1_d_cache.stat_paths;
    let stats = &ctx.sim.state.stats;
    let accesses = stats.get(paths.hits).unwrap_or(0.0) + stats.get(paths.misses).unwrap_or(0.0);
    assert_eq!(
        ctx.get_reg(A1 as usize),
        100 + 3 * (AMOS - 1),
        "{backend:?}: the last AMO's old value"
    );
    assert_eq!(
        ctx.sim.probe_mem_load(PhysAddr::new(DATA), 8),
        100 + 3 * AMOS,
        "{backend:?}: the word"
    );
    assert_eq!(accesses as u64, AMOS, "{backend:?}: L1D accesses for {AMOS} AMOs");
}

#[test]
fn an_amo_is_one_cache_access_inorder() {
    check_one_access_per_amo(BackendKind::InOrder);
}

#[test]
fn an_amo_is_one_cache_access_o3() {
    check_one_access_per_amo(BackendKind::OutOfOrder);
}

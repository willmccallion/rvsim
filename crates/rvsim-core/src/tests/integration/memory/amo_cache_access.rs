//! An AMO is one L1D access: the cache takes the line writable, performs
//! the read-modify-write and leaves the line modified, so commit has no
//! second store to send.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, A1, A2, X0};

const DATA: u64 = PROGRAM_BASE + 0x400;
const AMOS: u64 = 8;

/// `AMOS` `amoadd.d a1, a2, (a0)` on one word, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(A2, X0, 3).build(),
    ];
    program.extend((0..AMOS).map(|_| i().amoadd_d(A1, A0, A2).build()));
    program.push(i().jal(X0, 0).build());
    program
}

fn check_one_access_per_amo(backend: BackendKind) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.cache.wcb_entries = 0;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());
    ctx.sim.probe_mem_store(PhysAddr::new(DATA), 100, ProbeWidth::B8);

    ctx.run(400);

    let paths = &ctx.sim.state.cores[0].units.l1_d_cache.stat_paths;
    let stats = &ctx.sim.state.uncore.kernel.stats;
    let accesses = stats.get(paths.hits).unwrap_or(0.0) + stats.get(paths.misses).unwrap_or(0.0);
    assert_eq!(ctx.get_reg(A1), 100 + 3 * (AMOS - 1), "{backend:?}: the last AMO's old value");
    assert_eq!(
        ctx.sim.probe_mem_load(PhysAddr::new(DATA), ProbeWidth::B8),
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

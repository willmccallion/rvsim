//! An AMO takes effect in the cache, so it executes only as the oldest.
//!
//! One on a mispredicted path never reaches memory.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const BASE: u32 = 5;
const ONE: u32 = 6;
const SLOW: u32 = 7;
const DONE_REG: usize = 31;
const DONE: u64 = 7;

/// A branch that is taken once a divide finishes, skipping an `amoadd`
/// the front end fetches down the fall-through path meanwhile.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(BASE, 0).build(),
        i().addi(BASE, BASE, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(ONE, 0, 1).build(),
        i().div(SLOW, ONE, ONE).build(),
        i().bne(SLOW, 0, 8).build(),
        i().amoadd_d(0, BASE, ONE).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

#[test]
fn an_amo_on_a_mispredicted_path_never_reaches_memory() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut config = Config::default();
        config.pipeline.backend = backend;
        config.pipeline.width = 4;
        config.cache.l1_d.enabled = true;
        let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

        ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");

        let paths = ctx.sim.state.cores[0].units.stat_paths.bp;
        let mispredicts = ctx.sim.state.stats.get(paths.committed_mispredicts).unwrap_or(0.0);
        assert!(mispredicts >= 1.0, "{backend:?}: the branch was predicted not taken");
        assert_eq!(ctx.sim.probe_mem_load(PhysAddr::new(DATA), 8), 0, "{backend:?}");
    }
}

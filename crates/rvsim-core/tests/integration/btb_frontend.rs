//! Fetch knows a control instruction only through the BTB.
//!
//! One the BTB misses is found by decode, which redirects fetch, so a loop's
//! branches cost redirects until the BTB holds them.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const PROGRAM_BASE: u64 = 0x8000_0000;
const ITERATIONS: i32 = 20;
const DONE_REG: usize = 31;
const DONE: u64 = 7;

/// A loop whose body jumps twice before its backward branch.
fn looping_jumps() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let nop = i().nop().build();
    vec![
        i().addi(5, 0, ITERATIONS).build(),
        i().jal(0, 8).build(),
        nop,
        i().jal(0, 8).build(),
        nop,
        i().addi(5, 5, -1).build(),
        i().bne(5, 0, -20).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

fn decode_redirects(backend: BackendKind, btb_size: usize) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.btb_size = btb_size;
    config.pipeline.btb_ways = 1;
    config.cache.l1_i.enabled = true;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &looping_jumps());
    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    let path = ctx.sim.state.cores[0].units.stat_paths.bp.decode_redirects;
    ctx.sim.state.stats.get(path).unwrap_or(0.0) as u64
}

#[test]
fn a_loops_branches_redirect_from_decode_only_until_the_btb_holds_them() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let redirects = decode_redirects(backend, 4096);

        assert!(redirects <= 4, "{backend:?}: {redirects} decode redirects");
    }
}

#[test]
fn a_btb_too_small_for_the_loop_redirects_every_iteration() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let redirects = decode_redirects(backend, 1);

        assert!(redirects >= ITERATIONS as u64, "{backend:?}: {redirects} decode redirects");
    }
}

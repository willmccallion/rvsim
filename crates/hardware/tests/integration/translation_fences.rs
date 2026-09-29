//! Writing `satp` and executing `sfence.vma` act on address translation
//! only: the physically tagged caches keep their lines, as in gem5.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::{LineAddr, PhysAddr};
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;
use rvsim_core::sim::packet::MesiState;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const SATP: u32 = 0x180;
const SFENCE_VMA_ALL: u32 = 0x1200_0073;
/// `fence rw, rw`: the store drains before the maintenance instruction.
const FENCE_RW_RW: u32 = 0x0330_000F;
const SETTLE_NOPS: usize = 40;
const DONE_REG: usize = 2;
const DONE: u64 = 7;

/// Dirties the data line and lets the store drain, runs `maintenance`,
/// reloads the line and marks completion.
fn program(maintenance: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, 0x400).build(),
        i().sd(10, 0, 0).build(),
        FENCE_RW_RW,
    ];
    // Time for the drained store's write miss to fill the line.
    program.extend(std::iter::repeat_n(i().nop().build(), SETTLE_NOPS));
    program.extend([
        maintenance,
        i().ld(11, 10, 0).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

struct Outcome {
    data_line: Option<MesiState>,
    code_line_cached: bool,
}

fn run(backend: BackendKind, maintenance: u32) -> Outcome {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.cache.l1_i.enabled = true;
    config.cache.l1_d.enabled = true;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program(maintenance));

    ctx.run_until(3_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx.run(50);

    let units = &ctx.sim.state.cores[0].units;
    let line = LineAddr::from_phys(PhysAddr::new(DATA), 64);
    let data_line = units.l1_d_cache.held_lines().into_iter().find(|(l, _)| *l == line);
    let code_line_cached = units.l1_i_cache.contains(PROGRAM_BASE);
    Outcome { data_line: data_line.map(|(_, state)| state), code_line_cached }
}

fn assert_caches_untouched(maintenance: u32) {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let outcome = run(backend, maintenance);

        assert_eq!(outcome.data_line, Some(MesiState::Modified), "{backend:?}: dirty line kept");
        assert!(outcome.code_line_cached, "{backend:?}: code line kept");
    }
}

#[test]
fn a_satp_write_leaves_the_caches_alone() {
    assert_caches_untouched(InstructionBuilder::new().csrrw(0, SATP, 0).build());
}

#[test]
fn a_full_sfence_vma_leaves_the_caches_alone() {
    assert_caches_untouched(SFENCE_VMA_ALL);
}

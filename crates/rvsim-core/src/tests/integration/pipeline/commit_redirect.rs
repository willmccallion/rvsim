//! A redirect commit takes (a trap, an xRET, FENCE.I) reaches fetch the
//! cycle after: fetch issues the target's first entry the cycle after the
//! instruction that redirected it retires.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::{InstructionBuilder, MRET};
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const PROGRAM_BASE: u64 = 0x8000_0000;
const MEPC: u32 = 0x341;
/// Where MRET returns to, in the program's own line.
const TARGET: u64 = PROGRAM_BASE + 0x20;
const MRET_INDEX: u64 = 4;

/// Points `mepc` at `TARGET`, runs MRET, and spins at the target.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(T0, 0).build(),
        i().addi(T0, T0, (TARGET - PROGRAM_BASE) as i32).build(),
        i().csrrw(0, MEPC, T0).build(),
        MRET,
    ];
    program.resize(((TARGET - PROGRAM_BASE) / 4) as usize, i().nop().build());
    program.push(i().jal(0, 0).build());
    program
}

/// The cycle MRET retired and the cycle fetch first issued its target.
fn retire_and_refetch_cycles(backend: BackendKind) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 1;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    let mut retired = None;

    for _ in 0..500 {
        ctx.run(1);
        let cycle = ctx.sim.state.cycle;
        if retired.is_none() && ctx.sim.state.harts[0].instructions_retired == MRET_INDEX {
            retired = Some(cycle);
        }
        let Some(retired) = retired else { continue };
        let snapshot = ctx.sim.state.cores[0].pipeline.snapshot(1);
        if snapshot.fetch1_fetch2.iter().any(|entry| entry.pc == TARGET) {
            return (retired, cycle);
        }
    }
    panic!("{backend:?}: fetch never issued MRET's target");
}

#[test]
fn fetch_issues_an_mret_target_the_cycle_after_it_retires_inorder() {
    let (retired, refetched) = retire_and_refetch_cycles(BackendKind::InOrder);

    assert_eq!(refetched, retired + 1);
}

#[test]
fn fetch_issues_an_mret_target_the_cycle_after_it_retires_o3() {
    let (retired, refetched) = retire_and_refetch_cycles(BackendKind::OutOfOrder);

    assert_eq!(refetched, retired + 1);
}

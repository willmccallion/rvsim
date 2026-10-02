//! A drained pipeline leaves a self-contained architectural state.
//!
//! The hart sits at its committed PC and every committed store is in RAM,
//! whether it was still sitting in a store buffer or not.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const PROGRAM_BASE: u64 = 0x8000_0000;
const SLOTS: u64 = PROGRAM_BASE + 0x100;
const STORES: u64 = 32;
const MARK: u64 = 0x55;

/// Two setup instructions, then `STORES` back-to-back stores of `MARK` to
/// consecutive slots, then a spin. A wide commit retires stores faster
/// than the store buffer drains them.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().auipc(T0, 0).build(), i().addi(T1, 0, MARK as i32).build()];
    for slot in 0..STORES {
        program.push(i().sd(T0, T1, 0x100 + 8 * slot as i32).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

fn slot(ctx: &mut TestContext, index: u64) -> u64 {
    ctx.sim.probe_mem_load(PhysAddr::new(SLOTS + 8 * index), 8)
}

/// Runs `cycles`, drains, and returns how many committed stores the drain
/// still had to publish.
fn drain_after(backend: BackendKind, cycles: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    ctx.run(cycles);
    let committed = ctx.sim.state.harts[0].instructions_retired.saturating_sub(2).min(STORES);
    let in_ram_before = (0..committed).take_while(|&s| slot(&mut ctx, s) == MARK).count() as u64;

    ctx.sim.drain();

    assert_eq!(
        ctx.sim.state.cores[0].pipeline.fetch_pc(),
        ctx.sim.state.harts[0].pc,
        "{backend:?} @{cycles}: fetch restarts at the committed PC"
    );
    for s in 0..committed {
        assert_eq!(slot(&mut ctx, s), MARK, "{backend:?} @{cycles}: committed store {s} is in RAM");
    }
    if committed < STORES {
        assert_eq!(
            slot(&mut ctx, committed),
            0,
            "{backend:?} @{cycles}: no uncommitted store reached RAM"
        );
    }
    committed - in_ram_before
}

fn keeps_running_after_a_drain(backend: BackendKind, cycles: u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    ctx.run(cycles);
    ctx.sim.drain();
    let retired_at_drain = ctx.sim.state.harts[0].instructions_retired;

    ctx.run(200);

    let hart = &ctx.sim.state.harts[0];
    assert!(
        hart.instructions_retired > retired_at_drain,
        "{backend:?} @{cycles}: fetch resumed after the drain"
    );
    assert_eq!(
        hart.pc,
        PROGRAM_BASE + 4 * (STORES + 2),
        "{backend:?} @{cycles}: the program reached its spin"
    );
    for s in 0..STORES {
        assert_eq!(slot(&mut ctx, s), MARK, "{backend:?} @{cycles}: every store landed");
    }
}

#[test]
fn a_drained_pipeline_resumes_from_the_committed_pc() {
    for cycles in (5..60).step_by(7) {
        keeps_running_after_a_drain(BackendKind::InOrder, cycles);
        keeps_running_after_a_drain(BackendKind::OutOfOrder, cycles);
    }
}

#[test]
fn a_drain_leaves_the_committed_state_in_ram_on_both_backends() {
    for cycles in 5..60 {
        let _ = drain_after(BackendKind::InOrder, cycles);
        let _ = drain_after(BackendKind::OutOfOrder, cycles);
    }
}

/// The in-order commit retires up to four stores a cycle while the store
/// buffer drains one, so mid-burst drains find committed stores buffered.
#[test]
fn an_inorder_drain_publishes_stores_the_buffer_still_held() {
    let published_by_drain: u64 =
        (5..60).map(|cycles| drain_after(BackendKind::InOrder, cycles)).sum();

    assert!(published_by_drain > 0);
}

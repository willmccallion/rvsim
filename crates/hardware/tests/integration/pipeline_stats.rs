//! Pipeline stall and flush counters mean what their stats say: stalls in
//! cycles, and every applied flush once, under its cause.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T4: u32 = 29;

/// Loads that issue ahead of an older store to their address, whose
/// address waits on a divide: each is a memory-order violation until the
/// dependence predictor learns the pair. A branch on the same divide,
/// taken but first predicted not taken, resolves with them, so some
/// violations are found on its wrong path and squashed by its flush.
fn violating_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program =
        vec![i().auipc(T0, 1).build(), i().addi(T3, 0, 1).build(), i().addi(T1, 0, 7).build()];
    for _ in 0..8 {
        program.push(i().div(T2, T0, T3).build());
        program.push(i().beq(T2, T0, 8).build());
        program.push(i().addi(0, 0, 0).build());
        program.push(i().sw(T2, T1, 0).build());
        program.push(i().lw(T4, T0, 0).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

#[test]
fn flushes_by_cause_add_up_to_all_flushes() {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::OutOfOrder;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &violating_program());

    ctx.run(2000);

    let core_paths = &ctx.sim.state.cores[0].units.stat_paths;
    let paths = &core_paths.pipeline;
    let stats = &ctx.sim.state.stats;
    let count = |path| stats.get(path).unwrap_or(0.0);
    let by_cause = count(paths.flushes_branch)
        + count(paths.flushes_system)
        + count(paths.flushes_mem_violations);
    assert!(count(core_paths.mdp.violations) > 0.0, "the program violates memory order");
    assert_eq!(by_cause, count(paths.flushes_total));
    assert_eq!(ctx.get_reg(T4 as usize), 7, "the last load reads the stored value");
}

/// Independent divides compete for the one divider.
fn divide_bound_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(T0, 0, 100).build(), i().addi(T3, 0, 3).build()];
    for rd in 10..26 {
        program.push(i().div(rd, T0, T3).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

#[test]
fn fu_structural_stalls_are_counted_in_cycles() {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::OutOfOrder;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &divide_bound_program());

    ctx.run(60);

    let paths = &ctx.sim.state.cores[0].units.stat_paths.pipeline;
    let stats = &ctx.sim.state.stats;
    let stalls = stats.get(paths.stalls_fu_structural).unwrap_or(0.0);
    let cycles = stats.get(paths.cycles_total).unwrap_or(0.0);
    assert!(stalls > 0.0, "the divides wait for the divider");
    assert!(stalls <= cycles, "{stalls} stall cycles in {cycles} cycles");
}

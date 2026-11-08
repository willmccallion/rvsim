//! A divide squashed on a wrong path, or flushed by a trap, frees the
//! divider, as an iterative divider is killed with its instruction: the
//! next divide does not wait out the rest of the killed one.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, A1, A2, A3, S4, T0, T1, T2, T3, X0};
const HANDLER: u64 = PROGRAM_BASE + 0x100;
/// Below RAM, where nothing answers: a load from it faults.
const UNMAPPED: i32 = 0x100;

/// A branch that two multiplies resolve as taken, though not-taken was
/// predicted, with `wrong_path` behind it; the right path divides.
fn program(wrong_path: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(A0, X0, 1000).build(),
        i().addi(A1, X0, 10).build(),
        i().addi(T0, X0, 3).build(),
        i().addi(T1, X0, 5).build(),
        i().addi(T3, X0, 75).build(),
        i().mul(T2, T0, T1).build(),
        i().mul(T2, T2, T1).build(),
        i().beq(T2, T3, 12).build(),
        wrong_path,
        i().jal(X0, 0).build(),
        i().div(A3, A0, A1).build(),
        i().addi(S4, A3, -99).build(),
        i().jal(X0, 0).build(),
    ]
}

/// The cycle the right path's divide has written its result.
fn finish_cycle(backend: BackendKind, wrong_path: u32) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = 1;
    config.system.console = crate::config::Console::Quiet;
    let program = program(wrong_path);
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);

    let finished = ctx.run_until(2_000, |ctx| ctx.get_reg(S4) == 1);

    finished.unwrap_or_else(|| panic!("{backend:?}: the right path did not finish"))
}

#[test]
fn a_squashed_wrong_path_divide_does_not_delay_the_next_divide() {
    let i = InstructionBuilder::new;
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let after_nop = finish_cycle(backend, i().nop().build());

        let after_divide = finish_cycle(backend, i().div(A2, A0, A1).build());

        assert_eq!(after_divide, after_nop, "{backend:?}");
    }
}

/// A load that faults, with `behind` issued after it, then a spin; the trap
/// handler divides.
fn faulting_load_then(behind: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(A0, X0, 1000).build(),
        i().addi(A1, X0, 10).build(),
        i().addi(T0, X0, UNMAPPED).build(),
        i().ld(T1, T0, 0).build(),
        behind,
        i().jal(X0, 0).build(),
    ]
}

/// The handler: divide, then flag the result.
fn dividing_handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().div(A3, A0, A1).build(), i().addi(S4, A3, -99).build(), i().jal(X0, 0).build()]
}

/// The cycle the trap handler's divide has written its result.
fn handler_finish_cycle(backend: BackendKind, behind: u32) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = 1;
    config.system.console = crate::config::Console::Quiet;
    let program = faulting_load_then(behind);
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in dividing_handler().iter().enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(HANDLER + 4 * n as u64),
            u64::from(*word),
            ProbeWidth::B4,
        );
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    let finished = ctx.run_until(2_000, |ctx| ctx.get_reg(S4) == 1);

    finished.unwrap_or_else(|| panic!("{backend:?}: the handler did not finish"))
}

#[test]
fn a_divide_flushed_by_a_trap_does_not_delay_the_handlers_divide() {
    let i = InstructionBuilder::new;
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let after_nop = handler_finish_cycle(backend, i().nop().build());

        let after_divide = handler_finish_cycle(backend, i().div(A2, A0, A1).build());

        assert_eq!(after_divide, after_nop, "{backend:?}");
    }
}

//! `run_to` stops exactly where it was asked to: after a cycle or
//! instruction count, at a PC, or at a guest's break.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::{Config, Console};
use rvsim_core::sim::simulator::{StopAt, StopReason};

const PROGRAM_BASE: u64 = 0x8000_0000;
const SIM_CONTROL: u32 = 5;
const VALUE: u32 = 6;
const COUNTER: u32 = 7;
const BREAK_LABEL: i32 = 9;

/// A counted loop, a guest break labelled 9, then an exit with code 3.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(SIM_CONTROL, 0x102).build(),
        i().addi(COUNTER, 0, 100).build(),
        i().addi(COUNTER, COUNTER, -1).build(),
        i().bne(COUNTER, 0, -4).build(),
        i().addi(VALUE, 0, BREAK_LABEL).build(),
        i().sd(SIM_CONTROL, VALUE, 8).build(),
        i().addi(VALUE, 0, 4).build(),
        i().sd(SIM_CONTROL, VALUE, 0).build(),
        i().addi(VALUE, 0, 3).build(),
        i().sd(SIM_CONTROL, VALUE, 8).build(),
        i().addi(VALUE, 0, 3).build(),
        i().sd(SIM_CONTROL, VALUE, 0).build(),
        i().jal(0, 0).build(),
    ]
}

fn system() -> TestContext {
    let mut config = Config::default();
    config.system.console = Console::Quiet;
    TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program())
}

#[test]
fn a_cycle_count_stops_on_that_cycle() {
    let mut ctx = system();
    let start = ctx.sim.state.cycle;

    let reason =
        ctx.sim.run_to(&StopAt { cycles: Some(50), ..StopAt::default() }).expect("the run ticks");

    assert_eq!(reason, StopReason::Cycles);
    assert_eq!(ctx.sim.state.cycle - start, 50);
}

#[test]
fn an_instruction_count_stops_once_it_is_reached() {
    let mut ctx = system();

    let reason = ctx
        .sim
        .run_to(&StopAt { instructions: Some(40), ..StopAt::default() })
        .expect("the run ticks");

    assert_eq!(reason, StopReason::Instructions);
    let retired = ctx.sim.state.instructions_retired();
    assert!((40..44).contains(&retired), "{retired} retired");
}

#[test]
fn a_pc_stops_when_a_hart_is_about_to_run_it() {
    let mut ctx = system();
    let loop_exit = PROGRAM_BASE + 4 * 4;

    let reason = ctx
        .sim
        .run_to(&StopAt { pc: Some(loop_exit), ..StopAt::default() })
        .expect("the run ticks");

    assert_eq!(reason, StopReason::Pc { hart: 0 });
    assert_eq!(ctx.sim.state.harts[0].pc, loop_exit);
    assert_eq!(ctx.get_reg(COUNTER as usize), 0, "the loop has finished");
}

#[test]
fn a_guest_break_stops_the_run_and_it_resumes_to_the_exit() {
    let mut ctx = system();
    let everything = StopAt { cycles: Some(20_000), guest_breaks: true, ..StopAt::default() };

    let first = ctx.sim.run_to(&everything).expect("the run ticks");
    let second = ctx.sim.run_to(&everything).expect("the run ticks");

    assert_eq!(first, StopReason::GuestBreak { label: BREAK_LABEL as u64 });
    assert_eq!(second, StopReason::Exited(3));
}

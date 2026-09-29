//! `run_to` stops exactly where it was asked to: after a cycle or
//! instruction count, at a PC, at a guest's break, or when the console
//! prints.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::{Config, Console};
use rvsim_core::system::simulator::{StopAt, StopReason};

const PROGRAM_BASE: u64 = 0x8000_0000;
const SIM_CONTROL: u32 = 5;
const VALUE: u32 = 6;
const COUNTER: u32 = 7;
const BREAK_LABEL: i32 = 9;
const UART_BASE_UPPER: i32 = 0x1_0000;

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
fn any_of_the_pcs_stops_when_a_hart_is_about_to_run_it() {
    let mut ctx = system();
    let loop_exit = PROGRAM_BASE + 4 * 4;

    let reason = ctx
        .sim
        .run_to(&StopAt { pcs: vec![PROGRAM_BASE + 0x100, loop_exit], ..StopAt::default() })
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

/// Spins, prints `!` on the console, then spins forever.
fn printing_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(COUNTER, 0, 30).build(),
        i().addi(COUNTER, COUNTER, -1).build(),
        i().bne(COUNTER, 0, -4).build(),
        i().lui(SIM_CONTROL, UART_BASE_UPPER).build(),
        i().addi(VALUE, 0, i32::from(b'!')).build(),
        i().sb(SIM_CONTROL, VALUE, 0).build(),
        i().jal(0, 0).build(),
    ]
}

#[test]
fn console_output_stops_the_run_once_the_guest_prints() {
    let mut config = Config::default();
    config.system.console = Console::Captured;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &printing_program());
    let stop = StopAt { cycles: Some(20_000), console_output: true, ..StopAt::default() };

    let reason = ctx.sim.run_to(&stop).expect("the run ticks");

    assert_eq!(reason, StopReason::ConsoleOutput);
    assert_eq!(ctx.get_reg(COUNTER as usize), 0, "the spin before the print has finished");
    let uart = ctx.sim.state.bus.uart_mut().expect("the system has a console");
    assert_eq!(uart.take_output(), b"!");
}

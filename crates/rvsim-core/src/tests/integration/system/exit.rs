//! A guest's exit ends the run at the exit instruction, and the stores it
//! committed before exiting still take effect: the console prints all of
//! them, though the cycles spent finishing them are not counted.

use crate::Simulator;
use crate::config::{BackendKind, Config, Console};
use crate::system::SystemState;
use crate::system::simulator::{StopAt, StopReason};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const UART: u32 = 5;
const VALUE: u32 = 6;
const A0: u32 = 10;
const A7: u32 = 17;
const UART_BASE_UPPER: i32 = 0x1_0000;
const ECALL: u32 = 0x0000_0073;
const SYS_EXIT: i32 = 93;
const MESSAGE: &[u8] = b"fib(20)=6765\n";
const EXIT_CODE: i32 = 7;
const DEVICE_LATENCY: u64 = 100;

/// Prints `MESSAGE` on the console, then exits with `EXIT_CODE` straight
/// away, so the last characters are still in the store buffer at the exit.
fn printing_then_exiting() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().lui(UART, UART_BASE_UPPER).build()];
    for &byte in MESSAGE {
        program.push(i().addi(VALUE, 0, i32::from(byte)).build());
        program.push(i().sb(UART, VALUE, 0).build());
    }
    program.push(i().addi(A0, 0, EXIT_CODE).build());
    program.push(i().addi(A7, 0, SYS_EXIT).build());
    program.push(ECALL);
    program
}

/// A system whose console is as far away as the P550's and A72's, so the
/// stores to it are still in flight at the exit. Built without the test
/// harness, which makes the bus latency zero.
fn system(backend: BackendKind) -> TestContext {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = Console::Captured;
    config.system.bus_latency = DEVICE_LATENCY;
    let sim = Simulator::new(SystemState::build(&config, ""));
    let mut ctx = TestContext { sim }.load_program(PROGRAM_BASE, &printing_then_exiting());
    ctx.sim.set_direct_mode(true);
    ctx
}

fn console(ctx: &mut TestContext) -> Vec<u8> {
    ctx.sim.take_console_output()
}

#[test]
fn every_character_printed_before_the_exit_reaches_the_console() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = system(backend);

        let reason = ctx
            .sim
            .run_to(&StopAt { cycles: Some(20_000), ..StopAt::default() })
            .expect("the run ticks");

        assert_eq!(reason, StopReason::Exited(EXIT_CODE as u64), "{backend:?}");
        assert_eq!(
            String::from_utf8_lossy(&console(&mut ctx)),
            String::from_utf8_lossy(MESSAGE),
            "{backend:?}"
        );
    }
}

#[test]
fn the_run_ends_on_the_cycle_of_the_exit() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = system(backend);
        let exited_at = ctx.run_until(20_000, |ctx| ctx.sim.state.check_exit().is_some());

        let code = ctx.sim.take_exit();

        assert_eq!(code, Some(EXIT_CODE as u64), "{backend:?}");
        let exited_at = exited_at.expect("the guest exits");
        assert!(ctx.sim.state.cycle > exited_at, "{backend:?}: finishing the stores takes cycles");
        assert_eq!(ctx.sim.cycle(), exited_at, "{backend:?}");
        assert_eq!(ctx.sim.stats_window().0, exited_at, "{backend:?}");
    }
}

#[test]
fn a_run_resumed_after_the_exit_counts_the_cycles_since() {
    let mut ctx = system(BackendKind::InOrder);
    ctx.run_until(20_000, |ctx| ctx.sim.state.check_exit().is_some());
    ctx.sim.take_exit();

    ctx.sim.tick().expect("the run ticks");

    assert_eq!(ctx.sim.cycle(), ctx.sim.state.cycle);
}

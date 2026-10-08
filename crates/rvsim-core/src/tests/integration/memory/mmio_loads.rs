//! A load from a device register has a side effect, so it may only be
//! issued once it is the oldest instruction in the machine: nothing older
//! can still fault, redirect, or be interrupted around it. It reads the
//! device itself, never a buffered store to the same register.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::{FENCE_IORW, InstructionBuilder};
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const A0: u32 = 10;
const PROGRAM_BASE: u64 = 0x8000_0000;
const CHAIN: usize = 40;
/// The CLINT's `mtime`, which counts every cycle with `clint_divider = 1`.
const MTIME: u64 = 0x0200_BFF8;
/// The UART's transmit (write) and receive (read) register share an address.
const UART_THR_RBR: i32 = 0x1000_0000;
const DONE: u64 = 1;

/// A dependent chain of `CHAIN` adds that keeps the ROB head busy, then an
/// independent load of `mtime` that an out-of-order core could issue at
/// once.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..CHAIN).map(|_| i().addi(T0, T0, 1).build()).collect();
    program.extend([
        i().lui(A0, (MTIME >> 12) as i32 + 1).build(),
        i().addi(A0, A0, (MTIME & 0xFFF) as i32 - 0x1000).build(),
        i().ld(T1, A0, 0).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

fn mtime_seen_by_the_load(backend: BackendKind) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.clint_divider = 1;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    assert_eq!(ctx.get_reg(T0 as usize), CHAIN as u64, "{backend:?}: the chain retired");
    ctx.get_reg(T1 as usize)
}

#[test]
fn a_device_load_waits_until_it_is_the_oldest_instruction_o3() {
    let seen = mtime_seen_by_the_load(BackendKind::OutOfOrder);

    assert!(seen >= CHAIN as u64, "mtime {seen} was read before the {CHAIN}-add chain retired");
}

#[test]
fn a_device_load_waits_until_it_is_the_oldest_instruction_inorder() {
    let seen = mtime_seen_by_the_load(BackendKind::InOrder);

    assert!(seen >= CHAIN as u64, "mtime {seen} was read before the {CHAIN}-add chain retired");
}

/// Sends 'A' through the UART's transmit register, optionally fences, then
/// reads its receive register, which holds nothing: the read must see 0.
fn uart_receive_after_transmit(fenced: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, 0, i32::from(b'A')).build(),
        i().lui(A0, UART_THR_RBR >> 12).build(),
        i().sb(A0, T1, 0).build(),
    ];
    if fenced {
        program.push(FENCE_IORW);
    }
    program.extend([
        i().lbu(T2, A0, 0).build(),
        i().addi(T0, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

fn receive_register_read(backend: BackendKind, width: usize, fenced: bool) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = crate::config::Console::Quiet;
    let program = uart_receive_after_transmit(fenced);
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);

    let finished = ctx.run_until(2_000, |ctx| ctx.get_reg(T0 as usize) == DONE);

    assert!(finished.is_some(), "{backend:?} width {width}: the program did not finish");
    ctx.get_reg(T2 as usize)
}

#[test]
fn a_device_load_reads_the_device_not_a_buffered_store_to_it() {
    let mut nonzero = Vec::new();
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        for width in [1, 4] {
            for fenced in [false, true] {
                let read = receive_register_read(backend, width, fenced);
                if read != 0 {
                    nonzero.push(format!("{backend:?} width {width} fenced {fenced}: {read:#x}"));
                }
            }
        }
    }

    assert!(nonzero.is_empty(), "the receive register read a stored byte: {nonzero:#?}");
}

/// The cycle the instruction before [`program`]'s `mtime` load retired and
/// the cycle the load's device read was sent.
fn retire_and_device_read_cycles(backend: BackendKind) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 1;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    let older_than_load = CHAIN as u64 + 2;
    let mut older_retired = None;

    for _ in 0..400 {
        ctx.run(1);
        let cycle = ctx.sim.state.cycle;
        if older_retired.is_none() && ctx.sim.state.harts[0].instructions_retired == older_than_load
        {
            older_retired = Some(cycle);
        }
        if ctx.sim.state.cores[0].pipeline.device_read_in_flight() {
            let retired =
                older_retired.expect("the device read was sent before older ones retired");
            return (retired, cycle);
        }
    }
    panic!("{backend:?}: the device read was never sent");
}

#[test]
fn a_device_read_is_sent_the_cycle_after_the_last_older_instruction_retires() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (retired, sent) = retire_and_device_read_cycles(backend);

        assert_eq!(sent, retired + 1, "{backend:?}");
    }
}

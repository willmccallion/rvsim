//! A load from a device register has a side effect, so it may only be
//! issued once it is the oldest instruction in the machine: nothing older
//! can still fault, redirect, or be interrupted around it.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const A0: u32 = 10;
const PROGRAM_BASE: u64 = 0x8000_0000;
const CHAIN: usize = 40;
/// The CLINT's `mtime`, which counts every cycle with `clint_divider = 1`.
const MTIME: u64 = 0x0200_BFF8;

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

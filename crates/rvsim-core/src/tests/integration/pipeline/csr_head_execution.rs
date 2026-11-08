//! A CSR instruction executes only once everything before it has retired,
//! so what it reads is architectural: an instret read right after a run
//! of instructions counts every one of them.

use crate::config::BackendKind;
use crate::config::Config;
use crate::config::Console;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{T0, T1, X0};
const MINSTRET: u32 = 0xB02;
const ADDS: u64 = 40;

/// Forty adds, an instret read, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..ADDS).map(|_| i().addi(T0, T0, 1).build()).collect();
    program.push(i().csrrs(T1, MINSTRET, X0).build());
    program.push(i().jal(X0, 0).build());
    program
}

fn instret_seen(backend: BackendKind) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    assert_eq!(ctx.get_reg(T0), ADDS, "{backend:?}: the adds ran");
    ctx.get_reg(T1)
}

#[test]
fn an_instret_read_counts_every_instruction_before_it_inorder() {
    assert_eq!(instret_seen(BackendKind::InOrder), ADDS);
}

#[test]
fn an_instret_read_counts_every_instruction_before_it_o3() {
    assert_eq!(instret_seen(BackendKind::OutOfOrder), ADDS);
}

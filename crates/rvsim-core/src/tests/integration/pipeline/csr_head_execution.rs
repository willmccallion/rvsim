//! A CSR instruction executes only once everything before it has retired,
//! so what it reads is architectural: an instret read right after a run
//! of instructions counts every one of them.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const PROGRAM_BASE: u64 = 0x8000_0000;
const MINSTRET: u32 = 0xB02;
const ADDS: u64 = 40;

/// Forty adds, an instret read, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..ADDS).map(|_| i().addi(T0, T0, 1).build()).collect();
    program.push(i().csrrs(T1, MINSTRET, 0).build());
    program.push(i().jal(0, 0).build());
    program
}

fn instret_seen(backend: BackendKind) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    assert_eq!(ctx.get_reg(T0 as usize), ADDS, "{backend:?}: the adds ran");
    ctx.get_reg(T1 as usize)
}

#[test]
fn an_instret_read_counts_every_instruction_before_it_inorder() {
    assert_eq!(instret_seen(BackendKind::InOrder), ADDS);
}

#[test]
fn an_instret_read_counts_every_instruction_before_it_o3() {
    assert_eq!(instret_seen(BackendKind::OutOfOrder), ADDS);
}

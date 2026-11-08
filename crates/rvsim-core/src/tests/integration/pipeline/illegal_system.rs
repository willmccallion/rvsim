//! SYSTEM-opcode encodings the hart does not implement raise an
//! illegal-instruction exception instead of executing as something else.

use crate::config::BackendKind;
use crate::config::Config;
use crate::config::Console;
use crate::tests::support::builder::instruction::{InstructionBuilder, trap_recorder};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext, store_words};
use crate::tests::support::regs::{A2, A3, X0};

const HANDLER: u64 = PROGRAM_BASE + 0x100;
const ILLEGAL_INSTRUCTION: u64 = 2;
/// funct3=4 is reserved for the hypervisor loads and stores (HLV/HSV).
const SYSTEM_FUNCT3_4: u32 = 0x0000_4073;
/// `uret`, from the withdrawn N extension.
const URET: u32 = 0x0020_0073;

fn run(backend: BackendKind, inst: u32) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let program = [inst, InstructionBuilder::new().jal(X0, 0).build()];
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);
    store_words(&mut ctx.sim, HANDLER, &trap_recorder());
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    (ctx.get_reg(A2), ctx.get_reg(A3))
}

fn check(backend: BackendKind, inst: u32) {
    let (mcause, mtval) = run(backend, inst);

    assert_eq!(mcause, ILLEGAL_INSTRUCTION, "{backend:?} {inst:#010x}: illegal instruction");
    assert_eq!(mtval, u64::from(inst), "{backend:?} {inst:#010x}: tval holds the instruction");
}

#[test]
fn system_funct3_4_is_illegal_inorder() {
    check(BackendKind::InOrder, SYSTEM_FUNCT3_4);
}

#[test]
fn system_funct3_4_is_illegal_o3() {
    check(BackendKind::OutOfOrder, SYSTEM_FUNCT3_4);
}

#[test]
fn unimplemented_privileged_instruction_is_illegal_inorder() {
    check(BackendKind::InOrder, URET);
}

#[test]
fn unimplemented_privileged_instruction_is_illegal_o3() {
    check(BackendKind::OutOfOrder, URET);
}

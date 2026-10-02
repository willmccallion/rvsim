//! SYSTEM-opcode encodings the hart does not implement raise an
//! illegal-instruction exception instead of executing as something else.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const A2: u32 = 12;
const A3: u32 = 13;
const MCAUSE: u32 = 0x342;
const MTVAL: u32 = 0x343;
const ILLEGAL_INSTRUCTION: u64 = 2;
/// funct3=4 is reserved for the hypervisor loads and stores (HLV/HSV).
const SYSTEM_FUNCT3_4: u32 = 0x0000_4073;
/// `uret`, from the withdrawn N extension.
const URET: u32 = 0x0020_0073;

/// Records `mcause` and `mtval`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MTVAL, 0).build(), i().jal(0, 0).build()]
}

fn run(backend: BackendKind, inst: u32) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let program = [inst, InstructionBuilder::new().jal(0, 0).build()];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    (ctx.get_reg(A2 as usize), ctx.get_reg(A3 as usize))
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

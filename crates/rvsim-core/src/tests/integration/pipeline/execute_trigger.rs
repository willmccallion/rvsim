//! An execute trigger on an ordinary instruction raises a breakpoint
//! exception before that instruction retires, and `tdata1` takes the
//! debug spec's `mcontrol` layout.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const MCAUSE: u32 = 0x342;
const TDATA1: u32 = 0x7a1;
const MEPC: u32 = 0x341;
const BREAKPOINT: u64 = 3;
/// `mcontrol` type, execute match (bit 2), machine mode (bit 6),
/// breakpoint action.
const MCONTROL_EXECUTE_M: u64 = (2 << 60) | (1 << 2) | (1 << 6);
/// `mcontrol` with load, store and execute matches in M, S and U mode.
const MCONTROL_EVERY_ACCESS_AND_MODE: u64 = (2 << 60) | 0b101_1111;
const TCONTROL_MTE: u64 = 1 << 3;
const TRIGGERED_OFFSET: u64 = 4;

fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().addi(A1, 0, 1).build(), i().addi(A1, 0, 2).build(), i().jal(0, 0).build()]
}

/// Records `mcause` and `mepc`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MEPC, 0).build(), i().jal(0, 0).build()]
}

fn check(backend: BackendKind) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    for (n, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mtvec = HANDLER;
    csrs.tcontrol = TCONTROL_MTE;
    csrs.tdata1[0] = MCONTROL_EXECUTE_M;
    csrs.tdata2[0] = PROGRAM_BASE + TRIGGERED_OFFSET;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(500);

    assert_eq!(ctx.get_reg(A2 as usize), BREAKPOINT, "{backend:?}: mcause is breakpoint");
    assert_eq!(
        ctx.get_reg(A3 as usize),
        PROGRAM_BASE + TRIGGERED_OFFSET,
        "{backend:?}: mepc is the triggering instruction"
    );
    assert_eq!(
        ctx.get_reg(A1 as usize),
        1,
        "{backend:?}: the triggering instruction did not retire"
    );
}

#[test]
fn execute_trigger_raises_a_breakpoint_inorder() {
    check(BackendKind::InOrder);
}

#[test]
fn execute_trigger_raises_a_breakpoint_o3() {
    check(BackendKind::OutOfOrder);
}

/// Writes `MCONTROL_EVERY_ACCESS_AND_MODE` to `tdata1` and reads it back.
fn tdata1_after_writing_every_access_and_mode(backend: BackendKind) -> u64 {
    let i = InstructionBuilder::new;
    let mut config = Config::default();
    config.pipeline.backend = backend;
    let program =
        [i().csrrw(0, TDATA1, A1).build(), i().csrrs(A2, TDATA1, 0).build(), i().jal(0, 0).build()];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    ctx.set_reg(A1 as usize, MCONTROL_EVERY_ACCESS_AND_MODE);
    ctx.sim.sync_arch_regs();

    ctx.run(200);

    ctx.get_reg(A2 as usize)
}

#[test]
fn tdata1_keeps_the_mcontrol_fields_written_to_it() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let read_back = tdata1_after_writing_every_access_and_mode(backend);

        assert_eq!(read_back, MCONTROL_EVERY_ACCESS_AND_MODE, "{backend:?}");
    }
}

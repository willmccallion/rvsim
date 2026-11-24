//! An execute trigger on an ordinary instruction raises a breakpoint
//! exception before that instruction retires, and `tdata1` takes the
//! debug spec's `mcontrol` layout.

use crate::config::BackendKind;
use crate::config::Config;
use crate::config::Console;
use crate::config::CsrSquash;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext, store_words};
use crate::tests::support::regs::{A1, A2, A3, A4, A5, X0};

const HANDLER: u64 = PROGRAM_BASE + 0x100;
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
    vec![i().addi(A1, X0, 1).build(), i().addi(A1, X0, 2).build(), i().jal(X0, 0).build()]
}

/// Records `mcause` and `mepc`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, X0).build(), i().csrrs(A3, MEPC, X0).build(), i().jal(X0, 0).build()]
}

fn check(backend: BackendKind) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());
    store_words(&mut ctx.sim, HANDLER, &handler());
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mtvec = HANDLER;
    csrs.tcontrol = TCONTROL_MTE;
    csrs.tdata1[0] = MCONTROL_EXECUTE_M;
    csrs.tdata2[0] = PROGRAM_BASE + TRIGGERED_OFFSET;
    ctx.sim.sync_arch_regs();

    ctx.run(500);

    assert_eq!(ctx.get_reg(A2), BREAKPOINT, "{backend:?}: mcause is breakpoint");
    assert_eq!(
        ctx.get_reg(A3),
        PROGRAM_BASE + TRIGGERED_OFFSET,
        "{backend:?}: mepc is the triggering instruction"
    );
    assert_eq!(ctx.get_reg(A1), 1, "{backend:?}: the triggering instruction did not retire");
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
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let program = [
        i().csrrw(X0, TDATA1, A1).build(),
        i().csrrs(A2, TDATA1, X0).build(),
        i().jal(X0, 0).build(),
    ];
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);
    ctx.set_reg(A1, MCONTROL_EVERY_ACCESS_AND_MODE);
    ctx.sim.sync_arch_regs();

    ctx.run(200);

    ctx.get_reg(A2)
}

#[test]
fn tdata1_keeps_the_mcontrol_fields_written_to_it() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let read_back = tdata1_after_writing_every_access_and_mode(backend);

        assert_eq!(read_back, MCONTROL_EVERY_ACCESS_AND_MODE, "{backend:?}");
    }
}

const TDATA2: u32 = 0x7a2;
/// The instruction just behind the write that arms the trigger.
const ARMED_OFFSET: u64 = 8;

/// Arms an execute trigger with its own CSR writes on the instruction just
/// behind them: `tdata2` takes `a4`'s address, `tdata1` `a5`'s `mcontrol`.
fn arming_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().csrrw(X0, TDATA2, A4).build(),
        i().csrrw(X0, TDATA1, A5).build(),
        i().addi(A1, X0, 1).build(),
        i().addi(A1, X0, 2).build(),
        i().jal(X0, 0).build(),
    ]
}

/// `mcause`, `mepc` and `a1` once the program has armed its trigger.
fn armed_by_the_program(
    backend: BackendKind,
    width: usize,
    csr_squash: Option<CsrSquash>,
) -> (u64, u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.pipeline.csr_squash = csr_squash;
    config.system.console = Console::Quiet;
    let mut ctx =
        TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &arming_program());
    store_words(&mut ctx.sim, HANDLER, &handler());
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mtvec = HANDLER;
    csrs.tcontrol = TCONTROL_MTE;
    ctx.set_reg(A4, PROGRAM_BASE + ARMED_OFFSET);
    ctx.set_reg(A5, MCONTROL_EXECUTE_M);
    ctx.sim.sync_arch_regs();

    ctx.run(500);

    (ctx.get_reg(A2), ctx.get_reg(A3), ctx.get_reg(A1))
}

/// The instructions already fetched behind a trigger CSR write run under
/// the trigger it arms (#175): the in-order core refetches them after the
/// write, the out-of-order core holds them at rename until it is done.
#[test]
fn a_trigger_armed_by_a_csr_write_fires_on_the_next_instruction() {
    let machines = [
        (BackendKind::InOrder, None),
        (BackendKind::InOrder, Some(CsrSquash::AffectingWrites)),
        (BackendKind::OutOfOrder, None),
    ];
    for (backend, csr_squash) in machines {
        for width in [1, 4] {
            let (mcause, mepc, a1) = armed_by_the_program(backend, width, csr_squash);

            let context = format!("{backend:?} width {width} {csr_squash:?}");
            assert_eq!(mcause, BREAKPOINT, "{context}: mcause is breakpoint");
            assert_eq!(mepc, PROGRAM_BASE + ARMED_OFFSET, "{context}: mepc");
            assert_eq!(a1, 0, "{context}: the triggering instruction did not retire");
        }
    }
}

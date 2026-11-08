//! A WFI that wakes without taking an interrupt continues with exactly the
//! instructions behind it: none skipped, none run twice.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::{T0, T1, X0};

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const COUNTER: RegIdx = RegIdx::new(1);
const MARKER: RegIdx = RegIdx::new(2);
const MIE: u32 = 0x304;
const MIE_MSIE: i32 = 1 << 3;
const WFI: u32 = 0x1050_0073;
const ADDS: u32 = 5;

/// Raises the machine software interrupt with `mie.MSIE` set and
/// `mstatus.MIE` clear, so the WFI completes at once and no trap is taken,
/// then counts five instructions and sets a marker.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, X0, MIE_MSIE).build(),
        i().csrrs(X0, MIE, T1).build(),
        i().lui(T0, 0x02000).build(),
        i().addi(T1, X0, 1).build(),
        i().sw(T0, T1, 0).build(),
        WFI,
    ];
    program.extend((0..ADDS).map(|_| i().addi(COUNTER, COUNTER, 1).build()));
    program.push(i().addi(MARKER, X0, 1).build());
    program.push(i().jal(X0, 0).build());
    program
}

fn check(backend: BackendKind, width: usize) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());
    ctx.sim.probe_mem_store(
        PhysAddr::new(HANDLER),
        u64::from(InstructionBuilder::new().jal(X0, 0).build()),
        ProbeWidth::B4,
    );
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    let finished = ctx.run_until(5_000, |ctx| ctx.get_reg(MARKER) == 1);

    assert!(finished.is_some(), "{backend:?} w{width}: the program never reached its marker");
    assert_eq!(ctx.sim.state.harts[0].csrs.mcause, 0, "{backend:?} w{width}: no trap was taken");
    assert_eq!(
        ctx.get_reg(COUNTER),
        u64::from(ADDS),
        "{backend:?} w{width}: every instruction behind the WFI ran exactly once"
    );
}

#[test]
fn inorder_w1_continues_exactly_behind_a_wfi_that_wakes_at_once() {
    check(BackendKind::InOrder, 1);
}

#[test]
fn inorder_w4_continues_exactly_behind_a_wfi_that_wakes_at_once() {
    check(BackendKind::InOrder, 4);
}

#[test]
fn o3_w4_continues_exactly_behind_a_wfi_that_wakes_at_once() {
    check(BackendKind::OutOfOrder, 4);
}

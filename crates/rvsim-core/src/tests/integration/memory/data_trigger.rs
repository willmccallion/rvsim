//! A load or store trigger raises a breakpoint whose `mtval` is the address
//! accessed, as the privileged spec gives the faulting virtual address.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const DATA: u64 = PROGRAM_BASE + 0x1000;
const T0: u32 = 5;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const MCAUSE: u32 = 0x342;
const MTVAL: u32 = 0x343;
const BREAKPOINT: u64 = 3;
const TCONTROL_MTE: u64 = 1 << 3;
/// `mcontrol` matching loads (bit 0) in machine mode (bit 6).
const MCONTROL_LOAD_M: u64 = (2 << 60) | (1 << 0) | (1 << 6);
/// `mcontrol` matching stores (bit 1) in machine mode (bit 6).
const MCONTROL_STORE_M: u64 = (2 << 60) | (1 << 1) | (1 << 6);

/// Loads from or stores to `DATA`, then spins.
fn access(store: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let access = if store { i().sw(T0, 0, 0).build() } else { i().lw(A1, T0, 0).build() };
    vec![i().auipc(T0, 1).build(), access, i().jal(0, 0).build()]
}

/// Records `mcause` and `mtval`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MTVAL, 0).build(), i().jal(0, 0).build()]
}

/// The `mcause` and `mtval` a trigger armed with `tdata1` on `DATA` leaves.
fn trap_of_data_trigger(backend: BackendKind, store: bool, tdata1: u64) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &access(store));
    for (n, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mtvec = HANDLER;
    csrs.tcontrol = TCONTROL_MTE;
    csrs.tdata1[0] = tdata1;
    csrs.tdata2[0] = DATA;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(500);

    (ctx.get_reg(A2 as usize), ctx.get_reg(A3 as usize))
}

#[test]
fn a_load_trigger_reports_the_load_address_in_mtval() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let trap = trap_of_data_trigger(backend, false, MCONTROL_LOAD_M);

        assert_eq!(trap, (BREAKPOINT, DATA), "{backend:?}");
    }
}

#[test]
fn a_store_trigger_reports_the_store_address_in_mtval() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let trap = trap_of_data_trigger(backend, true, MCONTROL_STORE_M);

        assert_eq!(trap, (BREAKPOINT, DATA), "{backend:?}");
    }
}

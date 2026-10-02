//! Simultaneous interrupts: those destined for M-mode are taken before any
//! destined for S-mode, whatever their fixed-priority position.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const M_HANDLER: u64 = PROGRAM_BASE + 0x100;
const S_HANDLER: u64 = PROGRAM_BASE + 0x200;
const INTERRUPT: u64 = 1 << 63;
const SUPERVISOR_TIMER: u64 = 5;

/// A supervisor software interrupt delegated to S and a supervisor timer
/// interrupt left with M, both pending and enabled, on a hart in S-mode.
fn run(backend: BackendKind) -> (PrivilegeMode, u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    let spin = [InstructionBuilder::new().jal(0, 0).build()];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &spin);
    for handler in [M_HANDLER, S_HANDLER] {
        ctx.sim.probe_mem_store(PhysAddr::new(handler), u64::from(spin[0]), 4);
    }
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.privilege = PrivilegeMode::Supervisor;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.csrs.mtvec = M_HANDLER;
        hart.csrs.stvec = S_HANDLER;
        hart.csrs.mideleg = csr::MIP_SSIP;
        hart.csrs.mie = csr::MIE_SSIP | csr::MIE_STIE;
        hart.csrs.mstatus |= csr::MSTATUS_SIE;
        hart.csrs.mip = csr::MIP_SSIP | csr::MIP_STIP;
    }

    ctx.run(100);
    let hart = &ctx.sim.state.harts[0];
    (hart.privilege, hart.csrs.mcause, hart.csrs.scause)
}

#[test]
fn an_interrupt_for_m_mode_beats_a_higher_listed_one_for_s_mode() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (privilege, mcause, scause) = run(backend);

        assert_eq!(privilege, PrivilegeMode::Machine, "{backend:?}: taken to M");
        assert_eq!(mcause, INTERRUPT | SUPERVISOR_TIMER, "{backend:?}: the timer interrupt");
        assert_eq!(scause, 0, "{backend:?}: nothing taken to S first");
    }
}

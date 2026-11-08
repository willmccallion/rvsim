//! A load or store to a physical address no memory or device claims
//! raises an access fault in every privilege mode.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::BackendKind;
use crate::config::Config;
use crate::config::defaults::RAM_BASE;
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::{InstructionBuilder, JAL_SELF};
use crate::tests::support::harness::TestContext;
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{X0, X10, X11};

const RAM_SIZE: usize = 0x10_000;
const TRAP_PARK: u64 = RAM_BASE + 0x800;
const UNMAPPED: u64 = 0x5000_0000;
const LOAD_ACCESS_FAULT: u64 = 5;
const STORE_ACCESS_FAULT: u64 = 7;

/// Runs `x10 = UNMAPPED; <access> x10` in `privilege`, then `j .`.
fn run(backend: BackendKind, privilege: PrivilegeMode, access: u32) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let i = InstructionBuilder::new;
    let program = [i().lui(X10, (UNMAPPED >> 12) as i32).build(), access, JAL_SELF];
    let mut ctx = TestContext::with_fast_memory(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, inst) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(RAM_BASE + 4 * n as u64),
            u64::from(*inst),
            ProbeWidth::B4,
        );
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), ProbeWidth::B4);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.mtvec = TRAP_PARK;
        hart.privilege = privilege;
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(RAM_BASE);
    }
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    let csrs = &ctx.sim.state.harts[0].csrs;
    (csrs.mcause, csrs.mtval)
}

#[test]
fn a_machine_mode_load_from_an_unmapped_address_faults() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let load = InstructionBuilder::new().ld(X11, X10, 0).build();
        assert_eq!(
            run(backend, PrivilegeMode::Machine, load),
            (LOAD_ACCESS_FAULT, UNMAPPED),
            "{backend:?}"
        );
    }
}

#[test]
fn a_machine_mode_store_to_an_unmapped_address_faults() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let store = InstructionBuilder::new().sd(X10, X0, 0).build();
        assert_eq!(
            run(backend, PrivilegeMode::Machine, store),
            (STORE_ACCESS_FAULT, UNMAPPED),
            "{backend:?}"
        );
    }
}

#[test]
fn a_supervisor_mode_load_from_an_unmapped_address_faults() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let load = InstructionBuilder::new().ld(X11, X10, 0).build();
        assert_eq!(
            run(backend, PrivilegeMode::Supervisor, load),
            (LOAD_ACCESS_FAULT, UNMAPPED),
            "{backend:?}"
        );
    }
}

//! A load or store to a physical address no memory or device claims
//! raises an access fault in every privilege mode.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;
use rvsim_core::isa::privileged::PrivilegeMode;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x10_000;
const TRAP_PARK: u64 = RAM_BASE + 0x800;
const JAL_SELF: u32 = 0x0000_006F;
const UNMAPPED: u64 = 0x5000_0000;
const LOAD_ACCESS_FAULT: u64 = 5;
const STORE_ACCESS_FAULT: u64 = 7;

/// Runs `x10 = UNMAPPED; <access> x10` in `privilege`, then `j .`.
fn run(backend: BackendType, privilege: PrivilegeMode, access: u32) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.general.direct_mode = false;
    let i = InstructionBuilder::new;
    let program = [i().lui(10, (UNMAPPED >> 12) as i32).build(), access, JAL_SELF];
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, inst) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE + 4 * n as u64), u64::from(*inst), 4);
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), 4);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.mtvec = TRAP_PARK;
        hart.privilege = privilege;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = RAM_BASE;
    }
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    let csrs = &ctx.sim.state.harts[0].csrs;
    (csrs.mcause, csrs.mtval)
}

#[test]
fn a_machine_mode_load_from_an_unmapped_address_faults() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let load = InstructionBuilder::new().ld(11, 10, 0).build();
        assert_eq!(
            run(backend, PrivilegeMode::Machine, load),
            (LOAD_ACCESS_FAULT, UNMAPPED),
            "{backend:?}"
        );
    }
}

#[test]
fn a_machine_mode_store_to_an_unmapped_address_faults() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let store = InstructionBuilder::new().sd(10, 0, 0).build();
        assert_eq!(
            run(backend, PrivilegeMode::Machine, store),
            (STORE_ACCESS_FAULT, UNMAPPED),
            "{backend:?}"
        );
    }
}

#[test]
fn a_supervisor_mode_load_from_an_unmapped_address_faults() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let load = InstructionBuilder::new().ld(11, 10, 0).build();
        assert_eq!(
            run(backend, PrivilegeMode::Supervisor, load),
            (LOAD_ACCESS_FAULT, UNMAPPED),
            "{backend:?}"
        );
    }
}

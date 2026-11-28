//! `mstatus.MPRV`: a machine-mode load or store translates and is checked
//! at the privilege in `mstatus.MPP`, while instruction fetch stays at
//! machine mode.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::defaults::RAM_BASE;
use crate::config::{BackendKind, Config, Console};
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::{InstructionBuilder, JAL_SELF};
use crate::tests::support::harness::TestContext;
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{X10, X12};

const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const L0_PPN: u64 = 0x8_0102;
const CODE_PA: u64 = 0x8036_0000;
const DATA_VA: u64 = 0x4003_0000;
const DATA_PA: u64 = 0x8040_0000;
const TRAP_PARK: u64 = RAM_BASE + 0x100;
const PTE_V: u64 = 1;
const SUPERVISOR_PAGE: u64 = 0b1100_0111;
const LOADED: u64 = 0x1234;
const LOAD_PAGE_FAULT: u64 = 13;
const LOAD_OFFSET: i32 = 8;
const MPP_SUPERVISOR: u64 = 1;
const MPP_USER: u64 = 0;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, level: u32, pte: u64) {
    let index = (DATA_VA >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

/// Runs a machine-mode load from `DATA_VA`, which only a supervisor page
/// maps, with `mstatus.MPRV` set and `mstatus.MPP` = `mpp`. The code is
/// reached by physical address and no page maps it.
fn run_load_with_mprv(backend: BackendKind, mpp: u64) -> TestContext {
    let i = InstructionBuilder::new;
    let program =
        [i().lui(X10, (DATA_VA >> 12) as i32).build(), i().ld(X12, X10, LOAD_OFFSET).build()];
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, inst) in program.iter().chain(&[JAL_SELF]).enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(CODE_PA + 4 * n as u64),
            u64::from(*inst),
            ProbeWidth::B4,
        );
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), ProbeWidth::B4);
    ctx.sim.probe_mem_store(PhysAddr::new(DATA_PA + LOAD_OFFSET as u64), LOADED, ProbeWidth::B8);

    write_pte(&mut ctx, ROOT_PPN, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, 1, (L0_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L0_PPN, 0, ((DATA_PA >> 12) << 10) | SUPERVISOR_PAGE);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.csrs.mtvec = TRAP_PARK;
        hart.csrs.mstatus |= csr::MSTATUS_MPRV | (mpp << csr::MSTATUS_MPP_SHIFT);
        hart.privilege = PrivilegeMode::Machine;
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE_PA);
    }
    ctx.sim.sync_arch_regs();

    ctx.run(3_000);
    ctx
}

#[test]
fn a_machine_mode_load_with_mprv_translates_at_the_privilege_in_mpp() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let ctx = run_load_with_mprv(backend, MPP_SUPERVISOR);

        assert_eq!(ctx.sim.state.harts[0].csrs.mcause, 0, "{backend:?}: no trap");
        assert_eq!(ctx.get_reg(X12), LOADED, "{backend:?}");
    }
}

#[test]
fn a_machine_mode_load_with_mprv_is_checked_at_the_privilege_in_mpp() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let ctx = run_load_with_mprv(backend, MPP_USER);

        let csrs = &ctx.sim.state.harts[0].csrs;
        assert_eq!(
            (csrs.mcause, csrs.mtval),
            (LOAD_PAGE_FAULT, DATA_VA + LOAD_OFFSET as u64),
            "{backend:?}"
        );
    }
}

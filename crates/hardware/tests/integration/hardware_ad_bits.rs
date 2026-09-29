//! Hardware-managed A/D bits under Svadu.
//!
//! With `menvcfg.ADUE` set the page-table walker sets A when it walks a
//! leaf that lacks it, and a store sets D once it retires. With ADUE clear
//! the hart behaves as Svade.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr;
use rvsim_core::core::pipeline::engine::BackendType;
use rvsim_core::isa::privileged::mode::PrivilegeMode;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const L0_PPN: u64 = 0x8_0102;
const CODE_VA: u64 = 0x4002_0000;
const CODE_PA: u64 = 0x8036_0000;
const DATA_VA: u64 = 0x4003_0000;
const DATA_PA: u64 = 0x8040_0000;
/// The page after the data page, physically adjacent to it.
const NEXT_VA: u64 = DATA_VA + 0x1000;
const TRAP_PARK: u64 = RAM_BASE + 0x100;
const PTE_V: u64 = 1;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;
const PTE_RWX_AD: u64 = 0b1100_1111;
const PTE_X: u64 = 0b0000_1001;
const PTE_RW: u64 = 0b0000_0111;
const JAL_SELF: u32 = 0x0000_006F;
const STORED: u64 = 0x55;
const LOADED: u64 = 0x1234;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

fn leaf_pte(ctx: &mut TestContext, va: u64) -> u64 {
    let index = (va >> 12) & 0x1ff;
    ctx.sim.probe_mem_load(PhysAddr::new((L0_PPN << 12) | (index * 8)), 8)
}

/// Runs `program` then `j .` in supervisor mode on a Svadu hart with
/// `menvcfg` = `menvcfg`, the code page mapped with `code_flags` and the
/// data page with `data_flags`, neither in the TLB.
fn run_with(
    backend: BackendType,
    menvcfg: u64,
    program: &[u32],
    code_flags: u64,
    data_flags: u64,
) -> TestContext {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.isa.svadu = true;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, inst) in program.iter().chain(&[JAL_SELF]).enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(CODE_PA + 4 * n as u64), u64::from(*inst), 4);
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), 4);
    ctx.sim.probe_mem_store(PhysAddr::new(DATA_PA + 8), LOADED, 8);

    write_pte(&mut ctx, ROOT_PPN, CODE_VA, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, CODE_VA, 1, (L0_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L0_PPN, CODE_VA, 0, ((CODE_PA >> 12) << 10) | code_flags);
    write_pte(&mut ctx, L0_PPN, DATA_VA, 0, ((DATA_PA >> 12) << 10) | data_flags);
    write_pte(&mut ctx, L0_PPN, NEXT_VA, 0, (((DATA_PA + 0x1000) >> 12) << 10) | data_flags);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.csrs.mtvec = TRAP_PARK;
        hart.csrs.menvcfg = menvcfg;
        hart.privilege = PrivilegeMode::Supervisor;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = CODE_VA;
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(3_000);
    ctx
}

/// As [`run_with`] with hardware A/D updates enabled.
fn run(backend: BackendType, program: &[u32], code_flags: u64, data_flags: u64) -> TestContext {
    run_with(backend, csr::MENVCFG_ADUE, program, code_flags, data_flags)
}

fn store_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(10, (DATA_VA >> 12) as i32).build(),
        i().addi(11, 0, STORED as i32).build(),
        i().sd(10, 11, 0).build(),
    ]
}

/// A doubleword store whose last four bytes fall in the next page.
fn crossing_store_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(10, (NEXT_VA >> 12) as i32).build(),
        i().addi(11, 0, STORED as i32).build(),
        i().sd(10, 11, -4).build(),
    ]
}

fn load_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().lui(10, (DATA_VA >> 12) as i32).build(), i().ld(12, 10, 8).build()]
}

#[test]
fn a_store_to_a_clean_unaccessed_page_completes_and_sets_a_and_d() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run(backend, &store_program(), PTE_RWX_AD, PTE_RW);

        assert_eq!(ctx.sim.state.harts[0].csrs.mcause, 0, "{backend:?}: no trap");
        let stored = ctx.sim.probe_mem_load(PhysAddr::new(DATA_PA), 8);
        assert_eq!(stored, STORED, "{backend:?}: the store retired");
        assert_eq!(leaf_pte(&mut ctx, DATA_VA) & (PTE_A | PTE_D), PTE_A | PTE_D, "{backend:?}");
    }
}

#[test]
fn a_load_from_an_unaccessed_page_sets_a_but_not_d() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run(backend, &load_program(), PTE_RWX_AD, PTE_RW);

        assert_eq!(ctx.get_reg(12), LOADED, "{backend:?}: the load retired");
        assert_eq!(leaf_pte(&mut ctx, DATA_VA) & (PTE_A | PTE_D), PTE_A, "{backend:?}");
    }
}

#[test]
fn fetching_from_an_unaccessed_page_sets_its_a_bit() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run(backend, &load_program(), PTE_X, PTE_RW | PTE_A | PTE_D);

        assert_eq!(ctx.get_reg(12), LOADED, "{backend:?}: the code ran");
        assert_eq!(leaf_pte(&mut ctx, CODE_VA) & (PTE_A | PTE_D), PTE_A, "{backend:?}");
    }
}

#[test]
fn a_store_crossing_into_a_second_clean_page_sets_d_on_both() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run(backend, &crossing_store_program(), PTE_RWX_AD, PTE_RW);

        assert_eq!(ctx.sim.state.harts[0].csrs.mcause, 0, "{backend:?}: no trap");
        let stored = ctx.sim.probe_mem_load(PhysAddr::new(DATA_PA + 0xFFC), 8);
        assert_eq!(stored, STORED, "{backend:?}: the store retired");
        let pages = [leaf_pte(&mut ctx, DATA_VA), leaf_pte(&mut ctx, NEXT_VA)].map(|p| p & PTE_D);
        assert_eq!(pages, [PTE_D, PTE_D], "{backend:?}");
    }
}

#[test]
fn with_adue_clear_a_store_to_a_clean_page_raises_a_page_fault() {
    const STORE_PAGE_FAULT: u64 = 15;
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run_with(backend, 0, &store_program(), PTE_RWX_AD, PTE_RW);

        let csrs = &ctx.sim.state.harts[0].csrs;
        assert_eq!((csrs.mcause, csrs.mtval), (STORE_PAGE_FAULT, DATA_VA), "{backend:?}");
        assert_eq!(leaf_pte(&mut ctx, DATA_VA) & (PTE_A | PTE_D), 0, "{backend:?}: PTE untouched");
    }
}

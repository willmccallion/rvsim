//! A cache-block operation translates its block in the memory pipeline,
//! walking the page table on a TLB miss like a load or store, and reports
//! a fault as the store fault the CMO specification gives it.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr;
use rvsim_core::core::pipeline::engine::BackendType;
use rvsim_core::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
use rvsim_core::isa::encoding::zicboz::{CBO_CLEAN_IMM, CBO_ZERO_IMM, CBOZ_BLOCK_SIZE};
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
/// Where a trap to machine mode parks, on a jump to itself.
const TRAP_PARK: u64 = RAM_BASE + 0x100;
const PTE_V: u64 = 1;
const PTE_RWX_AD: u64 = 0b1100_1111;
const PTE_RW_AD: u64 = 0b1100_0111;
const PTE_X_A: u64 = 0b0100_1001;
const JAL_SELF: u32 = 0x0000_006F;
const X10: u32 = 10;
/// The CBO's operand: inside the data page's second block, not aligned.
const OPERAND: u64 = DATA_VA + 0x48;
const STORE_PAGE_FAULT: u64 = 15;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

const fn cbo(imm: i64, rs1: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM
}

/// Runs `x10 = OPERAND; <cbo> x10; j .` in supervisor mode with the data
/// page mapped with `data_pte_flags` and not yet in the TLB.
fn run(backend: BackendType, cbo_imm: i64, data_pte_flags: u64) -> TestContext {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    let i = InstructionBuilder::new;
    let program = [
        i().lui(X10, (DATA_VA >> 12) as i32).build(),
        i().addi(X10, X10, (OPERAND - DATA_VA) as i32).build(),
        cbo(cbo_imm, X10),
        JAL_SELF,
    ];
    for (n, inst) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(CODE_PA + 4 * n as u64), u64::from(*inst), 4);
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), 4);
    for off in (0..2 * CBOZ_BLOCK_SIZE).step_by(8) {
        ctx.sim.probe_mem_store(PhysAddr::new(DATA_PA + off), 0xDEAD_BEEF, 8);
    }

    write_pte(&mut ctx, ROOT_PPN, CODE_VA, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, CODE_VA, 1, (L0_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L0_PPN, CODE_VA, 0, ((CODE_PA >> 12) << 10) | PTE_RWX_AD);
    write_pte(&mut ctx, L0_PPN, DATA_VA, 0, ((DATA_PA >> 12) << 10) | data_pte_flags);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.csrs.menvcfg = csr::MENVCFG_CBZE | csr::MENVCFG_CBCFE;
        hart.csrs.mtvec = TRAP_PARK;
        hart.privilege = PrivilegeMode::Supervisor;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = CODE_VA;
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(2_000);
    ctx
}

#[test]
fn a_cbo_zero_that_misses_the_tlb_walks_and_zeroes_its_block() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut ctx = run(backend, CBO_ZERO_IMM, PTE_RW_AD);

        let hart = &ctx.sim.state.harts[0];
        assert_eq!(hart.csrs.mcause, 0, "{backend:?}: no trap");
        let zeroed = ctx.sim.probe_mem_load(PhysAddr::new(DATA_PA + CBOZ_BLOCK_SIZE), 8);
        let untouched = ctx.sim.probe_mem_load(PhysAddr::new(DATA_PA), 8);
        assert_eq!((zeroed, untouched), (0, 0xDEAD_BEEF), "{backend:?}");
    }
}

#[test]
fn a_cbo_clean_without_read_permission_raises_a_store_page_fault_at_its_operand() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let ctx = run(backend, CBO_CLEAN_IMM, PTE_X_A);

        let csrs = &ctx.sim.state.harts[0].csrs;
        assert_eq!((csrs.mcause, csrs.mtval), (STORE_PAGE_FAULT, OPERAND), "{backend:?}");
    }
}

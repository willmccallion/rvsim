//! A 32-bit instruction straddling a page boundary is fetched from both pages.
//!
//! Fetch1 translates the upper half-word, walking the page table when the
//! TLB is cold, and fetch2 reads it from that physical address.

use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr;
use rvsim_core::core::arch::mode::PrivilegeMode;
use rvsim_core::core::pipeline::engine::BackendType;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const CODE_L1_PPN: u64 = 0x8_0101;
const CODE_L0_PPN: u64 = 0x8_0102;
const CODE_VA: u64 = 0x4002_0000;
const FIRST_PA: u64 = 0x8036_0000;
const SECOND_PA: u64 = 0x8038_0000;
const A1: usize = 11;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;
/// `addi a1, zero, 42` split across the page boundary, then a jump to self.
const ADDI_A1_42: u32 = 0x02A0_0593;
const JAL_SELF: u32 = 0x0000_006F;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

fn run(backend: BackendType) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.uart_quiet = true;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    ctx.sim.probe_mem_store(PhysAddr::new(FIRST_PA + 0xFFE), u64::from(ADDI_A1_42 & 0xFFFF), 2);
    ctx.sim.probe_mem_store(PhysAddr::new(SECOND_PA), u64::from(ADDI_A1_42 >> 16), 2);
    ctx.sim.probe_mem_store(PhysAddr::new(SECOND_PA + 2), u64::from(JAL_SELF), 4);

    write_pte(&mut ctx, ROOT_PPN, (CODE_VA >> 30) & 0x1ff, (CODE_L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, CODE_L1_PPN, (CODE_VA >> 21) & 0x1ff, (CODE_L0_PPN << 10) | PTE_V);
    write_pte(
        &mut ctx,
        CODE_L0_PPN,
        (CODE_VA >> 12) & 0x1ff,
        ((FIRST_PA >> 12) << 10) | PTE_LEAF_RWX_AD,
    );
    write_pte(
        &mut ctx,
        CODE_L0_PPN,
        ((CODE_VA + 0x1000) >> 12) & 0x1ff,
        ((SECOND_PA >> 12) << 10) | PTE_LEAF_RWX_AD,
    );
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.privilege = PrivilegeMode::Supervisor;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = CODE_VA + 0xFFE;
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(2_000);
    ctx.get_reg(A1)
}

#[test]
fn an_instruction_straddling_two_pages_executes_inorder() {
    assert_eq!(run(BackendType::InOrder), 42);
}

#[test]
fn an_instruction_straddling_two_pages_executes_o3() {
    assert_eq!(run(BackendType::OutOfOrder), 42);
}

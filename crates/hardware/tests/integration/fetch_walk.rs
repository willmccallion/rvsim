//! A fetch parked on a page walk resumes at the instruction's real size.
//!
//! A compressed instruction whose translation needs a page-table walk must
//! not swallow the instruction after it once the walk returns.

use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr;
use rvsim_core::core::arch::mode::PrivilegeMode;
use rvsim_core::core::pipeline::engine::BackendType;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x40_0000;
const ROOT_PPN: u64 = 0x8_0100;
const NEXT_PPN: u64 = 0x8_0101;
const CODE: u64 = 0x8020_0000;
const A0: usize = 10;
const A1: usize = 11;

const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;

/// `c.li a0, 5` and `c.addi a0, 7` in one word, then `addi a1, a0, 0` and a
/// jump to self.
const PROGRAM: [u32; 3] = [0x051D_4515, 0x0005_0593, 0x0000_006F];

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

/// Identity-maps the code's 2 MiB megapage under Sv39 and runs from it in
/// supervisor mode with a cold TLB.
fn run_from_a_cold_tlb(backend: BackendType, width: usize) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.uart_quiet = true;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (i, word) in PROGRAM.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(CODE + (i as u64) * 4), u64::from(*word), 4);
    }
    write_pte(&mut ctx, ROOT_PPN, (CODE >> 30) & 0x1ff, (NEXT_PPN << 10) | PTE_V);
    write_pte(&mut ctx, NEXT_PPN, (CODE >> 21) & 0x1ff, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.privilege = PrivilegeMode::Supervisor;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = CODE;
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(2_000);
    (ctx.get_reg(A0), ctx.get_reg(A1))
}

#[test]
fn a_compressed_instruction_parked_on_a_walk_keeps_its_successor_inorder() {
    assert_eq!(run_from_a_cold_tlb(BackendType::InOrder, 1), (12, 12));
    assert_eq!(run_from_a_cold_tlb(BackendType::InOrder, 4), (12, 12));
}

#[test]
fn a_compressed_instruction_parked_on_a_walk_keeps_its_successor_o3() {
    assert_eq!(run_from_a_cold_tlb(BackendType::OutOfOrder, 1), (12, 12));
    assert_eq!(run_from_a_cold_tlb(BackendType::OutOfOrder, 4), (12, 12));
    assert_eq!(run_from_a_cold_tlb(BackendType::OutOfOrder, 10), (12, 12));
}

//! A fetch parked on a page walk resumes at the instruction's real size.
//!
//! A compressed instruction whose translation needs a page-table walk must
//! not swallow the instruction after it once the walk returns.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::BackendKind;
use crate::config::Config;
use crate::config::defaults::RAM_BASE;
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::harness::{TestContext, store_words};
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{A0, A1};

const RAM_SIZE: usize = 0x40_0000;
const ROOT_PPN: u64 = 0x8_0100;
const NEXT_PPN: u64 = 0x8_0101;
const CODE: u64 = 0x8020_0000;

const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;

/// `c.li a0, 5` and `c.addi a0, 7` in one word, then `addi a1, a0, 0` and a
/// jump to self.
const PROGRAM: [u32; 3] = [0x051D_4515, 0x0005_0593, 0x0000_006F];

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

/// Identity-maps the code's 2 MiB megapage under Sv39 and runs from it in
/// supervisor mode with a cold TLB.
fn run_from_a_cold_tlb(backend: BackendKind, width: usize) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).with_memory(RAM_SIZE, RAM_BASE);
    store_words(&mut ctx.sim, CODE, &PROGRAM);
    write_pte(&mut ctx, ROOT_PPN, (CODE >> 30) & 0x1ff, (NEXT_PPN << 10) | PTE_V);
    write_pte(&mut ctx, NEXT_PPN, (CODE >> 21) & 0x1ff, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.privilege = PrivilegeMode::Supervisor;
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE);
    }
    ctx.sim.sync_arch_regs();

    ctx.run(2_000);
    (ctx.get_reg(A0), ctx.get_reg(A1))
}

#[test]
fn a_compressed_instruction_parked_on_a_walk_keeps_its_successor_inorder() {
    assert_eq!(run_from_a_cold_tlb(BackendKind::InOrder, 1), (12, 12));
    assert_eq!(run_from_a_cold_tlb(BackendKind::InOrder, 4), (12, 12));
}

#[test]
fn a_compressed_instruction_parked_on_a_walk_keeps_its_successor_o3() {
    assert_eq!(run_from_a_cold_tlb(BackendKind::OutOfOrder, 1), (12, 12));
    assert_eq!(run_from_a_cold_tlb(BackendKind::OutOfOrder, 4), (12, 12));
    assert_eq!(run_from_a_cold_tlb(BackendKind::OutOfOrder, 10), (12, 12));
}

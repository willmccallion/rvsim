//! The walker caches a superpage leaf as one TLB entry covering the whole
//! page, as gem5's RISC-V TLB does, so one walk serves every base page in it.

use crate::support::builder::instruction::InstructionBuilder;
use crate::support::harness::TestContext;
use rvsim_core::common::{Asid, PhysAddr, Ppn, Vpn};
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;
use rvsim_core::isa::csr;
use rvsim_core::isa::privileged::PrivilegeMode;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const CODE: u64 = 0x8020_0000;
/// A 2 MiB megapage of data at this address, mapped to `DATA_PA`.
const DATA_VA: u64 = 0x4000_0000;
const DATA_PA: u64 = 0x8040_0000;
/// A base page inside the megapage the program never touches.
const UNTOUCHED_PAGE: u64 = DATA_VA + 0x1F_0000;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

#[test]
fn one_walk_of_a_megapage_translates_every_page_in_it() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut config = Config::default();
        config.pipeline.backend = backend;
        let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
        let i = InstructionBuilder::new;
        let program = [i().lui(10, (DATA_VA >> 12) as i32).build(), i().ld(11, 10, 0).build()];
        for (n, inst) in program.iter().chain(&[i().jal(0, 0).build()]).enumerate() {
            ctx.sim.probe_mem_store(PhysAddr::new(CODE + 4 * n as u64), u64::from(*inst), 4);
        }
        write_pte(&mut ctx, ROOT_PPN, CODE, 2, (L1_PPN << 10) | PTE_V);
        write_pte(&mut ctx, L1_PPN, CODE, 1, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
        write_pte(&mut ctx, ROOT_PPN, DATA_VA, 2, (L1_PPN << 10) | PTE_V);
        write_pte(&mut ctx, L1_PPN, DATA_VA, 1, ((DATA_PA >> 12) << 10) | PTE_LEAF_RWX_AD);
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

        ctx.run(500);

        let dtlb = &ctx.sim.state.cores[0].units.mmu.dtlb;
        let hit = dtlb.peek(Vpn::new(UNTOUCHED_PAGE >> 12), Asid::new(0)).map(|hit| hit.ppn);
        let expected = Ppn::new((DATA_PA + (UNTOUCHED_PAGE - DATA_VA)) >> 12);
        assert_eq!(hit, Some(expected), "{backend:?}: the megapage entry covers the page");
    }
}

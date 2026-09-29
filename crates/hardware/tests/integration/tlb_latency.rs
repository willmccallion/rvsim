//! An L2 TLB hit costs the configured latency before the access proceeds.
//!
//! Forty pages of loads evict the first page from a 32-entry L1 DTLB,
//! so the final load of page 0 hits the L2 TLB; that load, and nothing
//! else, gets slower when `l2_tlb_latency` grows.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;
use rvsim_core::isa::csr;
use rvsim_core::isa::privileged::PrivilegeMode;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const CODE_L1_PPN: u64 = 0x8_0101;
const DATA_L1_PPN: u64 = 0x8_0102;
const DATA_L0_PPN: u64 = 0x8_0103;
const CODE: u64 = 0x8020_0000;
const DATA_VA: u64 = 0x4000_0000;
const DATA_PA: u64 = 0x8030_0000;
const PAGES: u64 = 40;
const DRAIN_NOPS: u64 = 80;
const ROB_SIZE: usize = 4;
const L2_LATENCY: u64 = 25;
const L1_TLB_ENTRIES: usize = 32;
const L2_TLB_ENTRIES: usize = 512;
const L2_TLB_WAYS: usize = 4;
const A0: u32 = 10;
const A1: u32 = 11;
const T0: u32 = 5;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

/// Loads one word from each of `PAGES` pages, lets those walks drain
/// behind a run of NOPs, then loads from page 0 again and spins.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().lui(A0, (DATA_VA >> 12) as i32).build(), i().lui(T0, 1).build()];
    for _ in 0..PAGES {
        program.push(i().ld(A1, A0, 0).build());
        program.push(i().add(A0, A0, T0).build());
    }
    program.extend((0..DRAIN_NOPS).map(|_| i().nop().build()));
    program.push(i().lui(A0, (DATA_VA >> 12) as i32).build());
    program.push(i().ld(A1, A0, 0).build());
    program.push(i().jal(0, 0).build());
    program
}

fn cycles_to_finish(l2_tlb_latency: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::InOrder;
    config.pipeline.width = 1;
    // A tiny ROB keeps the commit backlog from hiding the delay.
    config.pipeline.rob_size = ROB_SIZE;
    config.memory.tlb_size = L1_TLB_ENTRIES;
    config.memory.l2_tlb_size = L2_TLB_ENTRIES;
    config.memory.l2_tlb_ways = L2_TLB_WAYS;
    config.memory.l2_tlb_latency = l2_tlb_latency;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    let program = program();
    for (i, word) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(CODE + (i as u64) * 4), u64::from(*word), 4);
    }
    let last_load_retired = program.len() as u64 - 1;
    write_pte(&mut ctx, ROOT_PPN, (CODE >> 30) & 0x1ff, (CODE_L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, CODE_L1_PPN, (CODE >> 21) & 0x1ff, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
    write_pte(&mut ctx, ROOT_PPN, (DATA_VA >> 30) & 0x1ff, (DATA_L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, DATA_L1_PPN, (DATA_VA >> 21) & 0x1ff, (DATA_L0_PPN << 10) | PTE_V);
    for page in 0..PAGES {
        let pa = DATA_PA + page * 0x1000;
        write_pte(&mut ctx, DATA_L0_PPN, page, ((pa >> 12) << 10) | PTE_LEAF_RWX_AD);
    }
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

    let mut cycles = 0;
    while ctx.sim.state.harts[0].instructions_retired < last_load_retired && cycles < 100_000 {
        ctx.run(1);
        cycles += 1;
    }
    assert!(cycles < 100_000, "every load retired");
    cycles
}

#[test]
fn an_l2_tlb_hit_delays_the_access_by_its_latency() {
    let fast = cycles_to_finish(0);
    let slow = cycles_to_finish(L2_LATENCY);

    let visible = L2_LATENCY - ROB_SIZE as u64;
    assert!(
        slow >= fast + visible,
        "l2_tlb_latency={L2_LATENCY} cost {} cycles over {fast}",
        slow - fast
    );
    assert!(
        slow < fast + L2_LATENCY * 4,
        "only the evicted pages pay the L2 latency, not all {PAGES}"
    );
}

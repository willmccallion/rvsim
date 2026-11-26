//! A load waiting in memory1 for an older store's data keeps the
//! translation it found, as hardware keeps a waiting access's physical
//! address: retrying it neither looks its page up in the TLB again nor
//! counts as another TLB hit.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::defaults::RAM_BASE;
use crate::config::{BackendKind, Config};
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{X0, X10, X11, X12, X13, X14, X15};

const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const CODE: u64 = 0x8020_0000;
/// A 2 MiB megapage of data at this address, mapped to `DATA_PA`.
const DATA_VA: u64 = 0x4000_0000;
const DATA_PA: u64 = 0x8040_0000;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;
const CYCLE_LIMIT: u64 = 2_000;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

/// The data TLB lookups, and the memory ops memory1 sent back to wait,
/// once the program retires: a load of the data page that fills the TLB,
/// then a store of what `make_data` computes and a load of the same
/// doubleword, whose addresses wait for the first load.
fn lookups_and_waits(backend: BackendKind, make_data: u32) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let mut ctx = TestContext::with_fast_memory(&config).with_memory(RAM_SIZE, RAM_BASE);
    let i = InstructionBuilder::new;
    let program = [
        i().lui(X10, (DATA_VA >> 12) as i32).build(),
        i().ld(X15, X10, 8).build(),
        i().add(X10, X10, X15).build(),
        i().addi(X12, X0, 100).build(),
        i().addi(X13, X0, 7).build(),
        make_data,
        i().sd(X10, X11, 0).build(),
        i().ld(X14, X10, 0).build(),
    ];
    for (n, inst) in program.iter().chain(&[i().jal(X0, 0).build()]).enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(CODE + 4 * n as u64),
            u64::from(*inst),
            ProbeWidth::B4,
        );
    }
    write_pte(&mut ctx, ROOT_PPN, CODE, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, CODE, 1, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
    write_pte(&mut ctx, ROOT_PPN, DATA_VA, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, DATA_VA, 1, ((DATA_PA >> 12) << 10) | PTE_LEAF_RWX_AD);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.privilege = PrivilegeMode::Supervisor;
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE);
    }
    ctx.sim.sync_arch_regs();
    let retired = program.len() as u64;

    ctx.run_until(CYCLE_LIMIT, |ctx| ctx.state().harts[0].instructions_retired >= retired)
        .expect("the program retired");

    let stats = &ctx.sim.state.uncore.kernel.stats;
    let stat = |path: &str| stats.get(path).unwrap_or(0.0) as u64;
    let lookups = stat("core0.mmu.dtlb.hits") + stat("core0.mmu.dtlb.misses");
    (lookups, stat("core0.lsq.rescheduled_mem_ops"))
}

/// Only the out-of-order backend sends a store's address ahead of its data,
/// so only there does a younger load reach memory1 before the data exists.
#[test]
fn a_load_waiting_for_store_data_looks_its_page_up_once() {
    let i = InstructionBuilder::new;
    let backend = BackendKind::OutOfOrder;

    let (lookups_ready, _) = lookups_and_waits(backend, i().addi(X11, X0, 14).build());
    let (lookups_waiting, waits) = lookups_and_waits(backend, i().div(X11, X12, X13).build());

    assert!(waits > 0, "the load waits for the divide's result");
    assert_eq!(lookups_waiting, lookups_ready);
}

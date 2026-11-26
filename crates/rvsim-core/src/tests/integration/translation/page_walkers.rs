//! Page-table walks on a core with a fixed number of walkers: a miss to a
//! page already being walked waits for that walk instead of walking it
//! again, and a miss that finds every walker busy waits for one.

use std::num::NonZeroUsize;

use crate::common::{PhysAddr, VirtAddr};
use crate::config::defaults::RAM_BASE;
use crate::config::{BackendKind, Config};
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{X0, X10, X11, X12, X13};

const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const CODE: u64 = 0x8020_0000;
/// Two 2 MiB megapages of data, which take a walk each. They share the
/// level-1 table with the code's megapage, in slots of their own.
const FIRST_VA: u64 = 0x4000_0000;
const SECOND_VA: u64 = 0x4040_0000;
const FIRST_PA: u64 = 0x8040_0000;
const SECOND_PA: u64 = 0x8060_0000;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;
const CYCLE_LIMIT: u64 = 2_000;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

fn map_megapage(ctx: &mut TestContext, va: u64, pa: u64) {
    write_pte(ctx, ROOT_PPN, va, 2, (L1_PPN << 10) | PTE_V);
    write_pte(ctx, L1_PPN, va, 1, ((pa >> 12) << 10) | PTE_LEAF_RWX_AD);
}

/// The walks the out-of-order core started, and the cycles it took, to run
/// `loads` with `walkers` page-table walkers. `x10` points at the first
/// data megapage and `x13` at the second.
fn walks_and_cycles(walkers: usize, loads: &[u32]) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.memory.page_walkers = NonZeroUsize::new(walkers).expect("a walker");
    let mut ctx = TestContext::with_fast_memory(&config).with_memory(RAM_SIZE, RAM_BASE);
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().lui(X10, (FIRST_VA >> 12) as i32).build(),
        i().lui(X13, (SECOND_VA >> 12) as i32).build(),
    ];
    program.extend(loads);
    for (n, inst) in program.iter().chain(&[i().jal(X0, 0).build()]).enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(CODE + 4 * n as u64),
            u64::from(*inst),
            ProbeWidth::B4,
        );
    }
    map_megapage(&mut ctx, CODE, CODE);
    map_megapage(&mut ctx, FIRST_VA, FIRST_PA);
    map_megapage(&mut ctx, SECOND_VA, SECOND_PA);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.privilege = PrivilegeMode::Supervisor;
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE);
    }
    ctx.sim.sync_arch_regs();
    let retired = program.len() as u64;

    let cycles = ctx
        .run_until(CYCLE_LIMIT, |ctx| ctx.state().harts[0].instructions_retired >= retired)
        .expect("the program retired");

    let walks = ctx.sim.state.uncore.kernel.stats.get("core0.mmu.walks").unwrap_or(0.0) as u64;
    (walks, cycles)
}

#[test]
fn loads_that_miss_on_one_page_together_walk_it_once() {
    let i = InstructionBuilder::new;
    let one_load = [i().ld(X11, X10, 0).build()];
    let two_loads = [i().ld(X11, X10, 0).build(), i().ld(X12, X10, 8).build()];

    let (walks_for_one, _) = walks_and_cycles(2, &one_load);
    let (walks_for_two, _) = walks_and_cycles(2, &two_loads);

    assert_eq!(walks_for_two, walks_for_one);
}

#[test]
fn with_one_walker_a_second_pages_walk_waits_for_the_first() {
    let i = InstructionBuilder::new;
    let loads = [i().ld(X11, X10, 0).build(), i().ld(X12, X13, 0).build()];

    let (walks_with_one, cycles_with_one) = walks_and_cycles(1, &loads);
    let (walks_with_two, cycles_with_two) = walks_and_cycles(2, &loads);

    assert_eq!(walks_with_one, walks_with_two, "every page is walked once either way");
    assert!(
        cycles_with_one > cycles_with_two,
        "one walker: {cycles_with_one} cycles, two: {cycles_with_two}"
    );
}

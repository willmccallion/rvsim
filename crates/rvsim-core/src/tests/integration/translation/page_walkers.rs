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

/// What the out-of-order core did running some loads.
struct Run {
    walks: u64,
    data_tlb_lookups: u64,
    cycles: u64,
}

/// The out-of-order core with `walkers` page-table walkers.
fn machine(walkers: usize) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.memory.page_walkers = NonZeroUsize::new(walkers).expect("a walker");
    config
}

/// Runs `loads` on `config`'s machine with every page of the program
/// mapped. `x10` points at the first data megapage and `x13` at the
/// second.
fn run(config: &Config, loads: &[u32]) -> Run {
    let mut ctx = TestContext::with_fast_memory(config).with_memory(RAM_SIZE, RAM_BASE);
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

    let stat = |path: &str| ctx.sim.state.uncore.kernel.stats.get(path).unwrap_or(0.0) as u64;
    let data_tlb_lookups = stat("core0.mmu.dtlb.hits") + stat("core0.mmu.dtlb.misses");
    Run { walks: stat("core0.mmu.walks"), data_tlb_lookups, cycles }
}

#[test]
fn loads_that_miss_on_one_page_together_walk_it_once() {
    let i = InstructionBuilder::new;
    let one_load = [i().ld(X11, X10, 0).build()];
    let two_loads = [i().ld(X11, X10, 0).build(), i().ld(X12, X10, 8).build()];

    let for_one = run(&machine(2), &one_load);
    let for_two = run(&machine(2), &two_loads);

    assert_eq!(for_two.walks, for_one.walks);
}

/// The load that waited for the walk of its page takes the translation
/// that walk left in the TLB, without looking its page up again.
#[test]
fn a_load_that_waited_for_its_pages_walk_looks_the_page_up_once() {
    let i = InstructionBuilder::new;
    let one_load = [i().ld(X11, X10, 0).build()];
    let two_loads = [i().ld(X11, X10, 0).build(), i().ld(X12, X10, 8).build()];

    let for_one = run(&machine(2), &one_load);
    let for_two = run(&machine(2), &two_loads);

    assert_eq!(for_two.data_tlb_lookups, for_one.data_tlb_lookups + 1);
}

#[test]
fn with_one_walker_a_second_pages_walk_waits_for_the_first() {
    let i = InstructionBuilder::new;
    let loads = [i().ld(X11, X10, 0).build(), i().ld(X12, X13, 0).build()];

    let with_one = run(&machine(1), &loads);
    let with_two = run(&machine(2), &loads);

    assert_eq!(with_one.walks, with_two.walks, "every page is walked once either way");
    assert!(
        with_one.cycles > with_two.cycles,
        "one walker: {} cycles, two: {}",
        with_one.cycles,
        with_two.cycles
    );
}

/// A walk that queued for the walker still waits out its own L2 TLB lookup
/// before it reads a PTE: with one walker it starts when two walkers would
/// start it, since the lookup outlasts the walk ahead of it.
#[test]
fn a_queued_walk_waits_out_its_l2_tlb_lookup() {
    const SLOW_L2_TLB: u64 = 40;
    const ADDRESS_CHAIN: usize = 10;
    let i = InstructionBuilder::new;
    let mut loads = vec![i().ld(X11, X10, 0).build()];
    loads.extend((0..ADDRESS_CHAIN).map(|_| i().addi(X13, X13, 0).build()));
    loads.push(i().ld(X12, X13, 0).build());
    let with = |walkers| {
        let mut config = machine(walkers);
        config.memory.l2_tlb_size = 64;
        config.memory.l2_tlb_latency = SLOW_L2_TLB;
        run(&config, &loads).cycles
    };

    assert_eq!(with(1), with(2));
}

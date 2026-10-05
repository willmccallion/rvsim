//! The load/store unit's prefetcher under Sv39, Sv48 and Sv57 paging: a
//! stream that runs off the end of a page stops there
//! (`PageBoundary::Stop`), or continues into the next page through the data
//! TLB (`PageBoundary::CrossWithTlb`), landing on that page's own frame,
//! never the frame after the current one.
//!
//! The two data pages' frames are not adjacent, so a prefetch that crossed
//! by physical address would fetch a line the program never maps. Under
//! Sv48 and Sv57 the data lies above Sv39's address range, so the walk uses
//! nonzero indices at the extra levels.

use std::collections::HashMap;

use crate::common::{Asid, LineAddr, PhysAddr, Ppn, Vpn};
use crate::config::{BackendKind, Config, LoadPrefetcherConfig, PageBoundary, PrefetcherKind};
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::uarch::mmu::tlb::PageSize;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
/// Page tables are allocated from here up.
const FIRST_TABLE_PPN: u64 = 0x8_0100;
const CODE: u64 = 0x8020_0000;
/// The frames of the two data pages, deliberately not adjacent.
const FIRST_FRAME: u64 = 0x8040_0000;
const SECOND_FRAME: u64 = 0x8060_5000;
/// The 1 GiB frame a gigapage of data maps to.
const GIGAPAGE_FRAME: u64 = 0x8000_0000;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;
const PTE_LEAF_X_A: u64 = 0b0100_1001;
const PMP_NAPOT: u8 = 0b0001_1000;
const PMP_RWX: u8 = 0b0000_0111;
const STRIDE: i32 = 256;
/// Loads across the whole first page, each address depending on the last
/// load's (zero) value so the misses do not fill every MSHR and leave the
/// prefetches none; a trained stream's last prefetches run four lines into
/// the second page.
const LOADS: i32 = 16;
const DATA_REG: usize = 10;
const NEXT_PAGE_REG: usize = 11;
const DONE_REG: usize = 2;
const DONE: u64 = 7;

#[derive(Clone, Copy, Debug)]
enum Mode {
    Sv39,
    Sv48,
    Sv57,
}

impl Mode {
    const ALL: [Self; 3] = [Self::Sv39, Self::Sv48, Self::Sv57];

    const fn levels(self) -> u32 {
        match self {
            Self::Sv39 => 3,
            Self::Sv48 => 4,
            Self::Sv57 => 5,
        }
    }

    const fn satp_mode(self) -> u64 {
        match self {
            Self::Sv39 => csr::SATP_MODE_SV39,
            Self::Sv48 => csr::SATP_MODE_SV48,
            Self::Sv57 => csr::SATP_MODE_SV57,
        }
    }

    /// The first data page: 1 GiB aligned, and above Sv39's range in the
    /// wider modes.
    const fn data_va(self) -> u64 {
        match self {
            Self::Sv39 => 0x0000_0000_4000_0000,
            Self::Sv48 => 0x0000_4000_4000_0000,
            Self::Sv57 => 0x00C0_4000_4000_0000,
        }
    }
}

/// How the data is mapped.
#[derive(Clone, Copy, Debug)]
enum Data {
    /// Two 4 KiB pages; the program loads from the second once before the
    /// loop, so the data TLB holds it.
    TouchedSecondPage,
    /// Two 4 KiB pages; the program never touches the second.
    UntouchedSecondPage,
    /// The second 4 KiB page is execute-only, and its translation is
    /// already in the data TLB.
    ExecuteOnlySecondPage,
    /// The second 4 KiB page is mapped readable and its translation is in
    /// the data TLB, but PMP denies its frame.
    PmpDeniedSecondPage,
    /// One 2 MiB megapage at `FIRST_FRAME`.
    Megapage,
    /// One 1 GiB gigapage at `GIGAPAGE_FRAME`.
    Gigapage,
}

impl Data {
    /// Where the first data page lands.
    const fn first_frame(self) -> u64 {
        match self {
            Self::Gigapage => GIGAPAGE_FRAME,
            _ => FIRST_FRAME,
        }
    }
}

/// Writes page tables into the simulated memory, allocating each table the
/// first time a walk needs it.
struct PageTables {
    root: u64,
    next_ppn: u64,
    children: HashMap<(u64, u64), u64>,
    levels: u32,
}

impl PageTables {
    fn new(levels: u32) -> Self {
        Self {
            root: FIRST_TABLE_PPN,
            next_ppn: FIRST_TABLE_PPN + 1,
            children: HashMap::new(),
            levels,
        }
    }

    /// Maps `va` to `pa` with a leaf at `leaf_level` (0 for 4 KiB).
    fn map(&mut self, ctx: &mut TestContext, va: u64, pa: u64, leaf_level: u32, flags: u64) {
        let mut table = self.root;
        for level in (leaf_level + 1..self.levels).rev() {
            let index = index_at(va, level);
            table = if let Some(&child) = self.children.get(&(table, index)) {
                child
            } else {
                let child = self.next_ppn;
                self.next_ppn += 1;
                write_pte(ctx, table, index, (child << 10) | PTE_V);
                let _ = self.children.insert((table, index), child);
                child
            };
        }
        write_pte(ctx, table, index_at(va, leaf_level), ((pa >> 12) << 10) | flags);
    }
}

/// `pmpaddr` for the naturally aligned power-of-two region of `bytes` at
/// `base`.
const fn napot(base: u64, bytes: u64) -> u64 {
    (base >> 2) | ((bytes >> 3) - 1)
}

const fn index_at(va: u64, level: u32) -> u64 {
    (va >> (12 + 9 * level)) & 0x1ff
}

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

fn program(touch_second_page: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let (data, next) = (DATA_REG as u32, NEXT_PAGE_REG as u32);
    let mut program = Vec::new();
    if touch_second_page {
        program.push(i().ld(13, next, 0).build());
    }
    program.extend([
        i().addi(6, 0, LOADS).build(),
        i().ld(7, data, 0).build(),
        i().add(data, data, 7).build(),
        i().addi(data, data, STRIDE).build(),
        i().addi(6, 6, -1).build(),
        i().bne(6, 0, -16).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

fn map_data(ctx: &mut TestContext, tables: &mut PageTables, data_va: u64, data: Data) {
    let next_va = data_va + 0x1000;
    match data {
        Data::Megapage => tables.map(ctx, data_va, FIRST_FRAME, 1, PTE_LEAF_RWX_AD),
        Data::Gigapage => tables.map(ctx, data_va, GIGAPAGE_FRAME, 2, PTE_LEAF_RWX_AD),
        Data::TouchedSecondPage
        | Data::UntouchedSecondPage
        | Data::ExecuteOnlySecondPage
        | Data::PmpDeniedSecondPage => {
            tables.map(ctx, data_va, FIRST_FRAME, 0, PTE_LEAF_RWX_AD);
            tables.map(ctx, next_va, SECOND_FRAME, 0, PTE_LEAF_RWX_AD);
        }
    }
}

/// What the L1D holds and the prefetcher counted after the loop.
struct Outcome {
    held: Vec<LineAddr>,
    page_boundary: f64,
    tlb_miss: f64,
    denied: f64,
}

impl Outcome {
    fn holds(&self, paddr: u64) -> bool {
        self.held.contains(&LineAddr::from_phys(PhysAddr::new(paddr), 64))
    }

    /// Lines of the page at `frame` past its first, which only a prefetch
    /// could have brought in.
    fn prefetched_into(&self, frame: u64) -> Vec<u64> {
        (1..4).map(|line| frame + line * 64 * 4).filter(|&paddr| self.holds(paddr)).collect()
    }
}

fn run(backend: BackendKind, mode: Mode, page_boundary: PageBoundary, data: Data) -> Outcome {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    // Large enough that the two pages' lines do not evict each other.
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.size_bytes = 32 * 1024;
    config.cache.l1_d.ways = 8;
    config.cache.l1_d.prefetcher = PrefetcherKind::None;
    config.cache.load_prefetcher =
        LoadPrefetcherConfig::Stride { table_size: 64, l1_lines: 4, l2_lines: 0, page_boundary };
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    let code = program(matches!(data, Data::TouchedSecondPage));
    for (n, inst) in code.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(CODE + 4 * n as u64), u64::from(*inst), 4);
    }
    let data_va = mode.data_va();
    let mut tables = PageTables::new(mode.levels());
    tables.map(&mut ctx, CODE, CODE, 1, PTE_LEAF_RWX_AD);
    map_data(&mut ctx, &mut tables, data_va, data);
    ctx.set_reg(DATA_REG, data_va);
    ctx.set_reg(NEXT_PAGE_REG, data_va + 0x1000);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (mode.satp_mode() << 60) | tables.root;
        hart.privilege = PrivilegeMode::Supervisor;
        if matches!(data, Data::PmpDeniedSecondPage) {
            hart.pmp.set_addr(0, napot(SECOND_FRAME, 0x1000));
            hart.pmp.set_cfg(0, PMP_NAPOT);
            hart.pmp.set_addr(1, u64::MAX >> 10);
            hart.pmp.set_cfg(1, PMP_NAPOT | PMP_RWX);
        } else {
            hart.pmp.set_addr(0, u64::MAX >> 10);
            hart.pmp.set_cfg(0, 0b0000_1111);
        }
        hart.pc = CODE;
    }
    let second_page_in_tlb = match data {
        Data::ExecuteOnlySecondPage => Some(PTE_LEAF_X_A),
        Data::PmpDeniedSecondPage => Some(PTE_LEAF_RWX_AD),
        _ => None,
    };
    if let Some(flags) = second_page_in_tlb {
        let vpn = Vpn::new(((data_va + 0x1000) >> 12) & ((1 << 44) - 1));
        let ppn = Ppn::new(SECOND_FRAME >> 12);
        let dtlb = &mut ctx.sim.state.cores[0].units.mmu.dtlb;
        dtlb.insert(vpn, ppn, flags | PTE_V, Asid::new(0), PageSize::Kib4);
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run_until(50_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("the loop finished");
    ctx.run(500);

    let held = ctx.sim.state.cores[0].units.l1_d_cache.held_lines();
    let stats = &ctx.sim.state.stats;
    let stat =
        |name: &str| stats.get(format!("core0.prefetch.loads.dropped.{name}")).unwrap_or(0.0);
    Outcome {
        held: held.into_iter().map(|(line, _)| line).collect(),
        page_boundary: stat("page_boundary"),
        tlb_miss: stat("tlb_miss"),
        denied: stat("denied"),
    }
}

/// Runs `check` on the outcome of every backend and paging mode.
fn for_every_machine(page_boundary: PageBoundary, data: Data, check: impl Fn(&Outcome, &str)) {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        for mode in Mode::ALL {
            let outcome = run(backend, mode, page_boundary, data);
            check(&outcome, &format!("{backend:?} {mode:?}"));
        }
    }
}

/// Lines just past the first frame: where a prefetch that crossed the page
/// by physical address would land.
const AFTER_FIRST_FRAME: u64 = FIRST_FRAME + 0x1000;

#[test]
fn crossing_with_the_tlb_prefetches_the_next_page_from_its_own_frame() {
    for_every_machine(PageBoundary::CrossWithTlb, Data::TouchedSecondPage, |outcome, machine| {
        assert!(!outcome.prefetched_into(SECOND_FRAME).is_empty(), "{machine}: nothing crossed");
        assert!(!outcome.holds(AFTER_FIRST_FRAME), "{machine}");
        assert!(outcome.prefetched_into(AFTER_FIRST_FRAME).is_empty(), "{machine}");
    });
}

#[test]
fn crossing_drops_a_prefetch_whose_page_misses_the_tlb() {
    for_every_machine(PageBoundary::CrossWithTlb, Data::UntouchedSecondPage, |outcome, machine| {
        assert!(outcome.tlb_miss > 0.0, "{machine}");
        assert!(!outcome.holds(SECOND_FRAME), "{machine}");
        assert!(outcome.prefetched_into(SECOND_FRAME).is_empty(), "{machine}");
        assert!(!outcome.holds(AFTER_FIRST_FRAME), "{machine}");
    });
}

#[test]
fn crossing_drops_a_prefetch_into_a_page_the_load_may_not_read() {
    for_every_machine(
        PageBoundary::CrossWithTlb,
        Data::ExecuteOnlySecondPage,
        |outcome, machine| {
            assert!(outcome.denied > 0.0, "{machine}");
            assert!(!outcome.holds(SECOND_FRAME), "{machine}");
            assert!(!outcome.holds(AFTER_FIRST_FRAME), "{machine}");
        },
    );
}

#[test]
fn crossing_drops_a_prefetch_into_a_frame_pmp_denies() {
    for_every_machine(PageBoundary::CrossWithTlb, Data::PmpDeniedSecondPage, |outcome, machine| {
        assert!(outcome.denied > 0.0, "{machine}");
        assert!(!outcome.holds(SECOND_FRAME), "{machine}");
        assert!(outcome.prefetched_into(SECOND_FRAME).is_empty(), "{machine}");
        assert!(!outcome.holds(AFTER_FIRST_FRAME), "{machine}");
    });
}

#[test]
fn stopping_keeps_prefetches_in_the_trained_page() {
    for_every_machine(PageBoundary::Stop, Data::TouchedSecondPage, |outcome, machine| {
        assert!(outcome.page_boundary > 0.0, "{machine}");
        assert!(outcome.prefetched_into(SECOND_FRAME).is_empty(), "{machine}");
        assert!(!outcome.holds(AFTER_FIRST_FRAME), "{machine}");
    });
}

#[test]
fn stopping_inside_a_megapage_does_not_stop_at_4k() {
    for_every_machine(PageBoundary::Stop, Data::Megapage, |outcome, machine| {
        assert_eq!(outcome.page_boundary, 0.0, "{machine}");
        let next_4k = Data::Megapage.first_frame() + 0x1000;
        assert!(!outcome.prefetched_into(next_4k).is_empty(), "{machine}");
    });
}

#[test]
fn stopping_inside_a_gigapage_does_not_stop_at_4k() {
    for_every_machine(PageBoundary::Stop, Data::Gigapage, |outcome, machine| {
        assert_eq!(outcome.page_boundary, 0.0, "{machine}");
        let next_4k = Data::Gigapage.first_frame() + 0x1000;
        assert!(!outcome.prefetched_into(next_4k).is_empty(), "{machine}");
    });
}

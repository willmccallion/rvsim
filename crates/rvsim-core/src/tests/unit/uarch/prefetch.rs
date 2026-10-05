//! The load/store unit's load prefetcher: per-PC streams kept a few lines
//! ahead in the L1D and further ahead in the L2, and where their
//! prefetches may go at a page boundary.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::PageBoundary;
use crate::sim::packet::CacheLevel;
use crate::uarch::mmu::PrefetchTranslation;
use crate::uarch::mmu::tlb::PageSize;
use crate::uarch::prefetch::{LoadPrefetch, LoadPrefetcher, PagePlacer, PrefetchDrop};

const LOAD: VirtAddr = VirtAddr(0x8000_1000);
const LINE: u64 = 64;

/// Accesses a load makes before its stride has saturated confidence.
const WARMUP: u64 = 5;

fn prefetcher(l1_lines: usize, l2_lines: usize) -> LoadPrefetcher {
    LoadPrefetcher::new(LINE as usize, 64, l1_lines, l2_lines, PageBoundary::Stop)
}

/// Places every line at the physical address equal to its virtual one.
fn identity() -> impl Fn(VirtAddr) -> Result<PhysAddr, PrefetchDrop> {
    |line| Ok(PhysAddr::new(line.val()))
}

/// Feeds `count` accesses `stride` bytes apart from `base`, every line
/// placeable, and returns what the last one sent.
fn walk(pf: &mut LoadPrefetcher, base: u64, stride: i64, count: u64) -> Vec<LoadPrefetch> {
    let mut last = Vec::new();
    for i in 0..count {
        let vaddr = VirtAddr::new(base.wrapping_add_signed(stride * i as i64));
        last = pf.train(LOAD, vaddr, identity());
    }
    last
}

fn lines(prefetches: &[LoadPrefetch], into: CacheLevel) -> Vec<u64> {
    prefetches.iter().filter(|p| p.into == into).map(|p| p.line.val()).collect()
}

#[test]
fn a_confident_stream_fills_the_l1d_near_and_the_l2_further_ahead() {
    let mut pf = prefetcher(2, 5);
    let base = 0x10_0000;

    let sent = walk(&mut pf, base, 256, WARMUP + 1);

    let at = base + 256 * WARMUP;
    assert_eq!(lines(&sent, CacheLevel::L1D), vec![at + 256, at + 512]);
    assert_eq!(lines(&sent, CacheLevel::L2), vec![at + 768, at + 1024, at + 1280]);
}

#[test]
fn each_later_access_sends_only_the_lines_not_yet_requested() {
    let mut pf = prefetcher(2, 5);
    let base = 0x10_0000;
    let _ = walk(&mut pf, base, 256, WARMUP + 1);

    let at = base + 256 * (WARMUP + 1);
    let sent = pf.train(LOAD, VirtAddr::new(at), identity());

    assert_eq!(lines(&sent, CacheLevel::L1D), vec![at + 512]);
    assert_eq!(lines(&sent, CacheLevel::L2), vec![at + 1280]);
}

#[test]
fn a_stride_shorter_than_a_line_steps_whole_lines() {
    let mut pf = prefetcher(2, 0);
    let base = 0x20_0000;

    let sent = walk(&mut pf, base, 8, WARMUP + 1);

    let line = (base + 8 * WARMUP) & !(LINE - 1);
    assert_eq!(lines(&sent, CacheLevel::L1D), vec![line + LINE, line + 2 * LINE]);
}

#[test]
fn no_l2_stream_when_it_reaches_no_further_than_the_l1d_stream() {
    let mut pf = prefetcher(4, 4);

    let sent = walk(&mut pf, 0x30_0000, 256, WARMUP + 1);

    assert_eq!(sent.len(), 4);
    assert!(lines(&sent, CacheLevel::L2).is_empty());
}

#[test]
fn a_level_stops_at_a_line_it_cannot_place_and_resumes_from_it() {
    let mut pf = prefetcher(4, 0);
    let base = 0x40_0000;
    let _ = walk(&mut pf, base, 256, WARMUP);
    let at = base + 256 * WARMUP;
    let fence = at + 3 * 256;
    let blocked = pf.train(LOAD, VirtAddr::new(at), |line| {
        if line.val() < fence { identity()(line) } else { Err(PrefetchDrop::PageBoundary) }
    });

    let next = pf.train(LOAD, VirtAddr::new(at + 256), identity());

    assert_eq!(lines(&blocked, CacheLevel::L1D), vec![at + 256, at + 512]);
    assert_eq!(lines(&next, CacheLevel::L1D), vec![at + 768, at + 1024, at + 1280]);
}

#[test]
fn walking_the_same_array_again_prefetches_it_again() {
    let mut pf = prefetcher(2, 0);
    let base = 0x50_0000;
    let _ = walk(&mut pf, base, 256, 64);

    let again = walk(&mut pf, base, 256, 3);

    assert_eq!(lines(&again, CacheLevel::L1D), vec![base + 768, base + 1024]);
}

const TRIGGER: VirtAddr = VirtAddr(0x1000_0100);
const TRIGGER_PADDR: PhysAddr = PhysAddr(0x8000_0100);

/// A data TLB holding the trigger's page, of `page`'s size, and the base
/// page after it at `next`, if any.
fn tlb(page: PageSize, next: Option<u64>) -> impl Fn(VirtAddr) -> PrefetchTranslation {
    move |vaddr: VirtAddr| {
        let page_mask = !(page.bytes() - 1);
        if vaddr.val() & page_mask == TRIGGER.val() & page_mask {
            let offset = vaddr.val() - (TRIGGER.val() & page_mask);
            return PrefetchTranslation::Mapped {
                paddr: PhysAddr::new((TRIGGER_PADDR.val() & page_mask) + offset),
                page,
            };
        }
        next.map_or(PrefetchTranslation::Missing, |paddr| PrefetchTranslation::Mapped {
            paddr: PhysAddr::new(paddr | (vaddr.val() & 0xFFF)),
            page: PageSize::Kib4,
        })
    }
}

#[test]
fn inside_the_trigger_page_a_prefetch_follows_the_trigger_translation() {
    let placer =
        PagePlacer::new(TRIGGER, TRIGGER_PADDR, PageBoundary::Stop, tlb(PageSize::Kib4, None));

    let placed = placer.place(VirtAddr::new(0x1000_0F00));

    assert_eq!(placed, Ok(PhysAddr::new(0x8000_0F00)));
}

#[test]
fn stop_refuses_the_next_page() {
    let placer = PagePlacer::new(
        TRIGGER,
        TRIGGER_PADDR,
        PageBoundary::Stop,
        tlb(PageSize::Kib4, Some(0x9000_0000)),
    );

    let placed = placer.place(VirtAddr::new(0x1000_1000));

    assert_eq!(placed, Err(PrefetchDrop::PageBoundary));
}

#[test]
fn stop_keeps_to_the_trigger_page_at_its_own_size() {
    let placer =
        PagePlacer::new(TRIGGER, TRIGGER_PADDR, PageBoundary::Stop, tlb(PageSize::Mib2, None));

    let placed = placer.place(VirtAddr::new(0x1000_5040));

    assert_eq!(placed, Ok(PhysAddr::new(0x8000_5040)));
}

#[test]
fn cross_with_tlb_takes_the_next_page_from_the_tlb() {
    let placer = PagePlacer::new(
        TRIGGER,
        TRIGGER_PADDR,
        PageBoundary::CrossWithTlb,
        tlb(PageSize::Kib4, Some(0x9000_0000)),
    );

    let placed = placer.place(VirtAddr::new(0x1000_1040));

    assert_eq!(placed, Ok(PhysAddr::new(0x9000_0040)));
}

#[test]
fn cross_with_tlb_drops_a_page_the_tlb_does_not_hold() {
    let placer = PagePlacer::new(
        TRIGGER,
        TRIGGER_PADDR,
        PageBoundary::CrossWithTlb,
        tlb(PageSize::Kib4, None),
    );

    let placed = placer.place(VirtAddr::new(0x1000_1040));

    assert_eq!(placed, Err(PrefetchDrop::TlbMiss));
}

#[test]
fn untranslated_accesses_stop_at_4k_or_cross_as_physical_addresses() {
    let physical = |_: VirtAddr| PrefetchTranslation::Untranslated;
    let trigger = VirtAddr::new(0x8000_0100);
    let stop = PagePlacer::new(trigger, PhysAddr::new(0x8000_0100), PageBoundary::Stop, physical);
    let cross =
        PagePlacer::new(trigger, PhysAddr::new(0x8000_0100), PageBoundary::CrossWithTlb, physical);

    let target = VirtAddr::new(0x8000_1040);

    assert_eq!(stop.place(target), Err(PrefetchDrop::PageBoundary));
    assert_eq!(cross.place(target), Ok(PhysAddr::new(0x8000_1040)));
}

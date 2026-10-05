//! The load/store unit's load prefetcher, after the Cortex-A72's (TRM
//! §6.4.9). It sits beside the load pipeline, so it trains on each load's
//! PC and virtual address; it keeps every confident stream a few lines
//! ahead in the L1D and further ahead in the L2 (`CPUECTLR_EL1[33:32]`), and
//! at a page boundary either stops or continues through the data TLB
//! (`CPUACTLR_EL1[43]`).

use crate::common::{PAGE_SHIFT, PhysAddr, VirtAddr};
use crate::config::PageBoundary;
use crate::sim::packet::CacheLevel;
use crate::soc::cache::prefetch::stride::{StrideTracker, line_along, pc_index};
use crate::uarch::mmu::PrefetchTranslation;

/// A prefetch the load prefetcher sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadPrefetch {
    /// The line's virtual address.
    pub line: VirtAddr,
    /// The line's physical address.
    pub paddr: PhysAddr,
    /// The cache it fills.
    pub into: CacheLevel,
}

/// Why a prefetch was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefetchDrop {
    /// It lies past the page of the trained load under `PageBoundary::Stop`.
    PageBoundary,
    /// Its page's translation is not in the data TLB.
    TlbMiss,
    /// The load could not read it.
    Denied,
    /// It is not RAM.
    NotRam,
}

/// The two streams a confident load keeps ahead of itself, nearest first.
const LEVELS: [CacheLevel; 2] = [CacheLevel::L1D, CacheLevel::L2];

/// One load's stream: what it has learned and how far ahead each level's
/// prefetches have already gone.
#[derive(Clone, Copy, Debug)]
struct Stream {
    pc: VirtAddr,
    tracker: StrideTracker,
    /// The furthest line requested into each of `LEVELS`.
    ahead: [Option<u64>; 2],
}

impl Stream {
    const fn new(pc: VirtAddr, vaddr: VirtAddr) -> Self {
        Self { pc, tracker: StrideTracker::starting_at(vaddr.val()), ahead: [None; 2] }
    }

    /// True unless level `level` has already requested `line`: its
    /// furthest request lies ahead of the access at `current`, within
    /// `reach` bytes of it, and at or beyond `line`. A request outside that
    /// window belongs to an earlier pass, so the stream starts again.
    const fn needs(&self, level: usize, line: u64, current: u64, stride: i64, reach: i64) -> bool {
        let Some(ahead) = self.ahead[level] else { return true };
        let direction = stride.signum();
        let lead = (ahead as i64).wrapping_sub(current as i64).wrapping_mul(direction);
        if lead <= 0 || lead > reach {
            return true;
        }
        (line as i64).wrapping_sub(ahead as i64).wrapping_mul(direction) > 0
    }
}

/// The load prefetcher: a reference prediction table indexed and tagged
/// by the load's PC, trained on virtual addresses.
#[derive(Debug)]
pub struct LoadPrefetcher {
    table: Vec<Option<Stream>>,
    table_mask: usize,
    line_bytes: u64,
    l1_lines: usize,
    l2_lines: usize,
    page_boundary: PageBoundary,
}

impl LoadPrefetcher {
    /// A prefetcher of `table_size` entries (a power of two, else 64) that
    /// keeps `l1_lines` lines ahead in the L1D and `l2_lines` in the L2.
    #[must_use]
    pub fn new(
        line_bytes: usize,
        table_size: usize,
        l1_lines: usize,
        l2_lines: usize,
        page_boundary: PageBoundary,
    ) -> Self {
        let size = if table_size.is_power_of_two() { table_size } else { 64 };
        Self {
            table: vec![None; size],
            table_mask: size - 1,
            line_bytes: line_bytes as u64,
            l1_lines,
            l2_lines,
            page_boundary,
        }
    }

    /// Whether a stream crosses into the next page.
    #[must_use]
    pub const fn page_boundary(&self) -> PageBoundary {
        self.page_boundary
    }

    /// Trains the stream of the load at `pc` on `vaddr` and returns the
    /// prefetches that keep a confident stream ahead, nearest first.
    /// `place` gives each line's physical address or why it cannot go; a
    /// level stops at its first line that cannot, and picks up from there
    /// on the load's next access.
    pub fn train(
        &mut self,
        pc: VirtAddr,
        vaddr: VirtAddr,
        mut place: impl FnMut(VirtAddr) -> Result<PhysAddr, PrefetchDrop>,
    ) -> Vec<LoadPrefetch> {
        let (line_bytes, l1_lines, l2_lines) = (self.line_bytes, self.l1_lines, self.l2_lines);
        let slot = &mut self.table[pc_index(pc, self.table_mask)];
        if slot.is_none_or(|stream| stream.pc != pc) {
            *slot = Some(Stream::new(pc, vaddr));
            return Vec::new();
        }
        let Some(stream) = slot.as_mut() else { return Vec::new() };
        let learned = stream.tracker.stride();
        let confident = stream.tracker.train(vaddr.val());
        if stream.tracker.stride() != learned {
            stream.ahead = [None; 2];
        }
        let Some(stride) = confident else { return Vec::new() };

        let ranges = [1..=l1_lines as i64, l1_lines as i64 + 1..=l2_lines as i64];
        let furthest = l1_lines.max(l2_lines) as i64 + 1;
        let reach = stride.abs().max(line_bytes as i64).saturating_mul(furthest);
        let mut prefetches = Vec::new();
        for (level, (into, steps)) in LEVELS.into_iter().zip(ranges).enumerate() {
            for k in steps {
                let line = line_along(vaddr.val(), stride, k, line_bytes);
                if !stream.needs(level, line, vaddr.val(), stride, reach) {
                    continue;
                }
                let Ok(paddr) = place(VirtAddr::new(line)) else { break };
                prefetches.push(LoadPrefetch { line: VirtAddr::new(line), paddr, into });
                stream.ahead[level] = Some(line);
            }
        }
        prefetches
    }
}

/// Decides where a prefetch for a load to `trigger` may go, by the page
/// the load's own translation lies in and the page-boundary policy.
#[derive(Debug)]
pub struct PagePlacer<F> {
    trigger: VirtAddr,
    trigger_paddr: PhysAddr,
    /// Bytes in the trigger's page; the smallest page when the data TLB no
    /// longer holds it.
    page_bytes: u64,
    translated: bool,
    page_boundary: PageBoundary,
    translate: F,
}

impl<F: Fn(VirtAddr) -> PrefetchTranslation> PagePlacer<F> {
    /// A placer for prefetches trained by a load to `trigger`, which went
    /// to `trigger_paddr`; `translate` looks a virtual address up in the
    /// data TLB.
    pub fn new(
        trigger: VirtAddr,
        trigger_paddr: PhysAddr,
        page_boundary: PageBoundary,
        translate: F,
    ) -> Self {
        let (page_bytes, translated) = match translate(trigger) {
            PrefetchTranslation::Mapped { page, .. } => (page.bytes(), true),
            PrefetchTranslation::Untranslated => (1 << PAGE_SHIFT, false),
            PrefetchTranslation::Missing | PrefetchTranslation::Denied => (1 << PAGE_SHIFT, true),
        };
        Self { trigger, trigger_paddr, page_bytes, translated, page_boundary, translate }
    }

    /// The physical address of `target`, or why it cannot be prefetched.
    /// Inside the trigger's page the address follows from the trigger's;
    /// past it, `Stop` refuses and `CrossWithTlb` asks the data TLB.
    pub fn place(&self, target: VirtAddr) -> Result<PhysAddr, PrefetchDrop> {
        let page_mask = !(self.page_bytes - 1);
        if target.val() & page_mask == self.trigger.val() & page_mask {
            let offset = target.val().wrapping_sub(self.trigger.val());
            return Ok(PhysAddr::new(self.trigger_paddr.val().wrapping_add(offset)));
        }
        match self.page_boundary {
            PageBoundary::Stop => Err(PrefetchDrop::PageBoundary),
            PageBoundary::CrossWithTlb if !self.translated => Ok(PhysAddr::new(target.val())),
            PageBoundary::CrossWithTlb => match (self.translate)(target) {
                PrefetchTranslation::Mapped { paddr, .. } => Ok(paddr),
                PrefetchTranslation::Untranslated => Ok(PhysAddr::new(target.val())),
                PrefetchTranslation::Missing => Err(PrefetchDrop::TlbMiss),
                PrefetchTranslation::Denied => Err(PrefetchDrop::Denied),
            },
        }
    }
}

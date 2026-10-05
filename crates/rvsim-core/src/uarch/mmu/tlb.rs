//! Translation Lookaside Buffers (TLBs).
//!
//! One structure serves both levels: `entries` slots in sets of `ways`
//! with LRU replacement within a set, fully associative when `ways` is zero
//! (gem5's RISC-V TLB, the default for the L1 TLBs) and absent when
//! `entries` is zero (the default for the shared L2 TLB, which gem5 does
//! not have). Each entry maps a whole page of the size its leaf PTE was
//! found at, so a 2 MiB kernel mapping is one entry; a page lives in the set
//! its page number above the page offset selects, and a lookup probes the
//! set of each page size.

use crate::common::{Asid, PAGE_SHIFT, Ppn, Vpn};

/// Bits of a VPN one page-table level translates.
const VPN_BITS_PER_LEVEL: u32 = 9;

/// PTE bits a TLB keeps: V, R, W, X, U, G and D.
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;
const PTE_G: u64 = 1 << 5;
const PTE_D: u64 = 1 << 7;

/// The size of the page a leaf PTE maps, from the level it was found at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    /// A base page, from a level-0 leaf.
    Kib4,
    /// A megapage, from a level-1 leaf.
    Mib2,
    /// A gigapage, from a level-2 leaf.
    Gib1,
    /// A terapage, from a level-3 leaf (Sv48 and Sv57).
    Gib512,
    /// A petapage, from a level-4 leaf (Sv57).
    Tib256,
}

impl PageSize {
    /// Every size, smallest first.
    pub const ALL: [Self; 5] = [Self::Kib4, Self::Mib2, Self::Gib1, Self::Gib512, Self::Tib256];

    /// The size a leaf found at walk level `level` maps; `None` beyond Sv57.
    #[must_use]
    pub const fn from_level(level: u32) -> Option<Self> {
        match level {
            0 => Some(Self::Kib4),
            1 => Some(Self::Mib2),
            2 => Some(Self::Gib1),
            3 => Some(Self::Gib512),
            4 => Some(Self::Tib256),
            _ => None,
        }
    }

    /// Low VPN bits that index within a page of this size.
    const fn vpn_offset_bits(self) -> u32 {
        let level = match self {
            Self::Kib4 => 0,
            Self::Mib2 => 1,
            Self::Gib1 => 2,
            Self::Gib512 => 3,
            Self::Tib256 => 4,
        };
        level * VPN_BITS_PER_LEVEL
    }

    /// Mask of the VPN bits that index within a page of this size.
    const fn vpn_offset_mask(self) -> u64 {
        (1u64 << self.vpn_offset_bits()) - 1
    }

    /// Bytes in a page of this size.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        1u64 << (PAGE_SHIFT as u32 + self.vpn_offset_bits())
    }
}

/// One cached translation: a page of `size` starting at base page `vpn`,
/// mapped to base physical page `ppn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// First base page of the virtual page.
    vpn: Vpn,
    /// First base page of the physical page.
    ppn: Ppn,
    /// Page size.
    size: PageSize,
    /// The leaf PTE's permission and status bits.
    pte: u64,
    /// Address space the mapping belongs to (ignored when global).
    asid: Asid,
}

impl Mapping {
    /// The mapping a leaf PTE `pte` gives for base page `vpn` (mapped to
    /// base page `ppn`) inside a page of `size`.
    const fn new(vpn: Vpn, ppn: Ppn, pte: u64, asid: Asid, size: PageSize) -> Self {
        let offset = vpn.val() & size.vpn_offset_mask();
        Self {
            vpn: Vpn::new(vpn.val() - offset),
            ppn: Ppn::new(ppn.val() - offset),
            size,
            pte,
            asid,
        }
    }

    /// The size of the page.
    #[must_use]
    pub const fn size(&self) -> PageSize {
        self.size
    }

    const fn is_global(&self) -> bool {
        self.pte & PTE_G != 0
    }

    /// True when the page contains base page `vpn`.
    const fn covers(&self, vpn: Vpn) -> bool {
        vpn.val() & !self.size.vpn_offset_mask() == self.vpn.val()
    }

    /// True when the page contains `vpn` in address space `asid`.
    const fn translates(&self, vpn: Vpn, asid: Asid) -> bool {
        self.covers(vpn) && (self.is_global() || self.asid.val() == asid.val())
    }

    /// The hit for base page `vpn`, which the mapping covers.
    const fn hit(self, vpn: Vpn) -> TlbHit {
        let pte = self.pte;
        TlbHit {
            ppn: Ppn::new(self.ppn.val() + (vpn.val() - self.vpn.val())),
            r: pte & PTE_R != 0,
            w: pte & PTE_W != 0,
            x: pte & PTE_X != 0,
            u: pte & PTE_U != 0,
            d: pte & PTE_D != 0,
            mapping: self,
        }
    }
}

/// Translation data and permission bits returned on a TLB hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TlbHit {
    /// Physical page of the looked-up base page.
    pub ppn: Ppn,
    /// Read permission.
    pub r: bool,
    /// Write permission.
    pub w: bool,
    /// Execute permission.
    pub x: bool,
    /// User-mode accessible.
    pub u: bool,
    /// Dirty bit (if `false` on a write, the PTW must set it before the mapping is cached).
    pub d: bool,
    /// The whole mapping, for promoting an L2 hit into an L1 TLB.
    pub mapping: Mapping,
}

/// The organisation of one TLB level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlbGeometry {
    /// Entry count; zero for no TLB at this level.
    pub entries: usize,
    /// Ways per set; zero for fully associative.
    pub ways: usize,
}

/// A slot of a TLB.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    mapping: Option<Mapping>,
    /// Use sequence number of the last access (larger is more recent;
    /// gem5's `lruSeq`).
    last_use: u64,
}

/// A TLB of `sets * ways` slots with LRU replacement within a set.
#[derive(Debug)]
pub struct Tlb {
    /// Slots laid out set by set.
    slots: Vec<Slot>,
    /// Associativity (ways per set).
    ways: usize,
    /// Mask for set indexing (`num_sets - 1`).
    set_mask: usize,
    /// Last use sequence number handed out.
    next_use: u64,
}

impl Tlb {
    /// A TLB of `entries` slots in sets of `ways`: fully associative when
    /// `ways` is zero or covers every entry, and holding nothing when
    /// `entries` is zero. The set count is rounded up to a power of two.
    #[must_use]
    pub fn new(geometry: TlbGeometry) -> Self {
        let TlbGeometry { entries, ways } = geometry;
        if entries == 0 {
            return Self { slots: Vec::new(), ways: 0, set_mask: 0, next_use: 0 };
        }
        let ways = if ways == 0 || ways >= entries { entries } else { ways };
        let num_sets = entries.div_ceil(ways).next_power_of_two();
        Self {
            slots: vec![Slot::default(); num_sets * ways],
            ways,
            set_mask: num_sets - 1,
            next_use: 0,
        }
    }

    /// The slots of the set a page of `size` containing `vpn` lives in.
    const fn set_of(&self, vpn: Vpn, size: PageSize) -> std::ops::Range<usize> {
        let page_number = vpn.val() >> size.vpn_offset_bits();
        let base = ((page_number as usize) & self.set_mask) * self.ways;
        base..base + self.ways
    }

    /// The slot translating base page `vpn` in address space `asid`.
    fn find(&self, vpn: Vpn, asid: Asid) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        PageSize::ALL.into_iter().find_map(|size| {
            self.set_of(vpn, size).find(|&i| {
                self.slots[i].mapping.is_some_and(|m| m.size == size && m.translates(vpn, asid))
            })
        })
    }

    /// Looks up base page `vpn` in address space `asid`, marking the entry
    /// most recently used.
    pub fn lookup(&mut self, vpn: Vpn, asid: Asid) -> Option<TlbHit> {
        let index = self.find(vpn, asid)?;
        self.next_use += 1;
        self.slots[index].last_use = self.next_use;
        self.slots[index].mapping.map(|m| m.hit(vpn))
    }

    /// Looks up base page `vpn` without touching replacement state, for
    /// observers outside the pipeline.
    #[must_use]
    pub fn peek(&self, vpn: Vpn, asid: Asid) -> Option<TlbHit> {
        self.find(vpn, asid).and_then(|index| self.slots[index].mapping).map(|m| m.hit(vpn))
    }

    /// Caches the translation of base page `vpn` to `ppn` inside a page of
    /// `size` described by leaf `pte`.
    pub fn insert(&mut self, vpn: Vpn, ppn: Ppn, pte: u64, asid: Asid, size: PageSize) {
        self.insert_mapping(Mapping::new(vpn, ppn, pte, asid, size));
    }

    /// Caches `mapping`, replacing one for the same page, else an empty or
    /// the least recently used way of its set.
    pub fn insert_mapping(&mut self, mapping: Mapping) {
        if self.slots.is_empty() {
            return;
        }
        let set = self.set_of(mapping.vpn, mapping.size);
        let same_page =
            |m: Mapping| m.vpn == mapping.vpn && m.size == mapping.size && m.asid == mapping.asid;
        let index = set
            .clone()
            .find(|&i| self.slots[i].mapping.is_some_and(same_page))
            .or_else(|| set.clone().find(|&i| self.slots[i].mapping.is_none()))
            .or_else(|| set.clone().min_by_key(|&i| self.slots[i].last_use))
            .unwrap_or(set.start);
        self.next_use += 1;
        self.slots[index] = Slot { mapping: Some(mapping), last_use: self.next_use };
    }

    fn remove_if(&mut self, doomed: impl Fn(&Mapping) -> bool) {
        for slot in &mut self.slots {
            if slot.mapping.as_ref().is_some_and(&doomed) {
                slot.mapping = None;
            }
        }
    }

    /// Drops the entries covering base page `vpn` in any address space
    /// (used for the dirty-bit re-walk).
    pub fn invalidate(&mut self, vpn: Vpn) {
        self.remove_if(|m| m.covers(vpn));
    }

    /// Flushes every entry (SFENCE.VMA with rs1=x0, rs2=x0).
    pub fn flush(&mut self) {
        self.remove_if(|_| true);
    }

    /// Flushes the entries covering `vpn` in any address space (SFENCE.VMA
    /// with rs1!=x0, rs2=x0).
    pub fn flush_vaddr(&mut self, vpn: Vpn) {
        self.remove_if(|m| m.covers(vpn));
    }

    /// Flushes the non-global entries of `asid` (SFENCE.VMA with rs1=x0,
    /// rs2!=x0).
    pub fn flush_asid(&mut self, asid: Asid) {
        self.remove_if(|m| !m.is_global() && m.asid == asid);
    }

    /// Flushes the non-global entries of `asid` covering `vpn` (SFENCE.VMA
    /// with rs1!=x0, rs2!=x0).
    pub fn flush_vaddr_asid(&mut self, vpn: Vpn, asid: Asid) {
        self.remove_if(|m| m.covers(vpn) && !m.is_global() && m.asid == asid);
    }
}

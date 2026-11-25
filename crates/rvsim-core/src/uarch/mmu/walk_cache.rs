//! The walker's cache of pointer PTEs (the Cortex-A72's "intermediate table
//! walk caches", Rocket's PTE cache): each entry remembers which page table
//! the walk reaches below some level for a range of virtual addresses, so a
//! walk that hits starts at that table instead of at the root. It holds no
//! leaf, so it never translates an address by itself; it is fully
//! associative with LRU replacement. As the privileged spec allows, an
//! SFENCE.VMA naming an address leaves it alone (that orders only leaf
//! PTEs), and one naming no address flushes it.

use crate::common::{Asid, PAGE_SHIFT, VirtAddr};

/// Bits of a virtual address each page-table level translates.
const VPN_BITS_PER_LEVEL: u64 = 9;

/// A page table a walk can start at: the table read at `level`, whose
/// physical page is `ppn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableStart {
    /// The level of the table, counting down to 0 at the leaves.
    pub level: u32,
    /// The table's physical page number.
    pub ppn: u64,
}

/// One cached pointer: the table at `table.level` serving every virtual
/// address whose bits above that level's index equal `vpn_prefix`.
#[derive(Clone, Copy, Debug)]
struct Entry {
    table: TableStart,
    vpn_prefix: u64,
    asid: Asid,
    /// The pointer PTE set G, so the table serves every address space.
    is_global: bool,
    last_use: u64,
}

impl Entry {
    const fn serves(&self, vaddr: VirtAddr, asid: Asid) -> bool {
        self.vpn_prefix == vpn_prefix(vaddr, self.table.level)
            && (self.is_global || self.asid.val() == asid.val())
    }
}

/// The virtual-address bits translated above the table at `level`.
const fn vpn_prefix(vaddr: VirtAddr, level: u32) -> u64 {
    vaddr.val() >> (PAGE_SHIFT + (level as u64 + 1) * VPN_BITS_PER_LEVEL)
}

/// A pointer PTE a walk read, to cache.
#[derive(Clone, Copy, Debug)]
pub struct WalkedPointer {
    /// The address being translated.
    pub vaddr: VirtAddr,
    /// The address space the walk ran in.
    pub asid: Asid,
    /// The table the pointer leads to.
    pub table: TableStart,
    /// The pointer PTE set G.
    pub is_global: bool,
}

/// A fully associative cache of `capacity` pointer PTEs; none when zero.
#[derive(Debug)]
pub struct WalkCache {
    entries: Vec<Entry>,
    capacity: usize,
    next_use: u64,
}

impl WalkCache {
    /// A cache of `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity), capacity, next_use: 0 }
    }

    /// Whether the machine has a walk cache at all.
    #[must_use]
    pub const fn exists(&self) -> bool {
        self.capacity > 0
    }

    /// The deepest table cached for `vaddr` in `asid`, marking it most
    /// recently used.
    pub fn lookup(&mut self, vaddr: VirtAddr, asid: Asid) -> Option<TableStart> {
        let index = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.serves(vaddr, asid))
            .min_by_key(|(_, entry)| entry.table.level)
            .map(|(index, _)| index)?;
        self.next_use += 1;
        self.entries[index].last_use = self.next_use;
        Some(self.entries[index].table)
    }

    /// Caches `pointer`, replacing the entry for the same table and range,
    /// else the least recently used one when full.
    pub fn insert(&mut self, pointer: WalkedPointer) {
        if !self.exists() {
            return;
        }
        self.next_use += 1;
        let entry = Entry {
            table: pointer.table,
            vpn_prefix: vpn_prefix(pointer.vaddr, pointer.table.level),
            asid: pointer.asid,
            is_global: pointer.is_global,
            last_use: self.next_use,
        };
        let same_range = self.entries.iter().position(|cached| {
            cached.table.level == entry.table.level
                && cached.vpn_prefix == entry.vpn_prefix
                && cached.asid == entry.asid
        });
        match same_range.or_else(|| self.victim()) {
            Some(index) => self.entries[index] = entry,
            None => self.entries.push(entry),
        }
    }

    /// The least recently used entry, once the cache is full.
    fn victim(&self) -> Option<usize> {
        if self.entries.len() < self.capacity {
            return None;
        }
        self.entries.iter().enumerate().min_by_key(|(_, entry)| entry.last_use).map(|(i, _)| i)
    }

    /// Flushes every entry (SFENCE.VMA with rs1=x0, rs2=x0).
    pub fn flush(&mut self) {
        self.entries.clear();
    }

    /// Flushes the non-global entries of `asid` (SFENCE.VMA with rs1=x0,
    /// rs2!=x0).
    pub fn flush_asid(&mut self, asid: Asid) {
        self.entries.retain(|entry| entry.is_global || entry.asid != asid);
    }
}

//! Translation Lookaside Buffer (TLB).
//!
//! Provides both L1 TLBs (direct-mapped, per iTLB/dTLB) and a shared L2 TLB
//! (set-associative). L1 TLBs cache the mapping between Virtual Page Numbers
//! (VPN) and Physical Page Numbers (PPN), along with permission bits (R/W/X/U)
//! to speed up address translation. On L1 miss the shared L2 TLB is consulted
//! before invoking the hardware page table walker.

/// A single entry in the TLB.
#[derive(Clone, Copy, Default)]
struct TlbEntry {
    /// Virtual Page Number (Tag).
    vpn: u64,
    /// Physical Page Number (Data).
    ppn: u64,
    /// Entry validity flag.
    valid: bool,
    /// Read permission.
    r: bool,
    /// Write permission.
    w: bool,
    /// Execute permission.
    x: bool,
    /// User mode accessible.
    u: bool,
    /// Dirty bit from PTE.
    d: bool,
    /// Address Space Identifier from SATP[59:44].
    asid: u16,
    /// PTE Global bit — matches regardless of ASID.
    global: bool,
}

/// Translation Lookaside Buffer structure.
pub struct Tlb {
    /// Vector of TLB entries.
    entries: Vec<TlbEntry>,
    /// Mask used for indexing (size - 1).
    mask: usize,
}

impl Tlb {
    /// Creates a new TLB with the specified size.
    ///
    /// # Arguments
    ///
    /// * `size` - Number of entries (will be rounded up to next power of 2).
    pub fn new(size: usize) -> Self {
        let safe_size = if size.is_power_of_two() {
            size
        } else {
            size.next_power_of_two()
        };

        Self {
            entries: vec![TlbEntry::default(); safe_size],
            mask: safe_size - 1,
        }
    }

    /// Looks up a VPN in the TLB.
    ///
    /// # Arguments
    ///
    /// * `vpn` - The Virtual Page Number to look up.
    /// * `asid` - The current Address Space Identifier from SATP[59:44].
    ///
    /// # Returns
    ///
    /// `Some((ppn, r, w, x, u, d))` if found, otherwise `None`.
    /// Global entries (G bit set in PTE) match regardless of ASID.
    ///
    /// # Panics
    ///
    /// This function will not panic. The unsafe array access is guaranteed safe because:
    /// - `idx = vpn & self.mask` where `mask = size - 1` (size is power of 2)
    /// - This ensures `idx` is always `< size` and within bounds of `entries`
    #[inline(always)]
    pub fn lookup(&self, vpn: u64, asid: u16) -> Option<(u64, bool, bool, bool, bool, bool)> {
        let idx = (vpn as usize) & self.mask;

        // SAFETY: idx is guaranteed to be < entries.len() by the mask operation above.
        // The mask is constructed as (size - 1) where size is the length of entries,
        // ensuring idx is always a valid index.
        let entry = unsafe { self.entries.get_unchecked(idx) };

        if entry.valid && entry.vpn == vpn && (entry.global || entry.asid == asid) {
            return Some((entry.ppn, entry.r, entry.w, entry.x, entry.u, entry.d));
        }
        None
    }

    /// Inserts a new mapping into the TLB.
    ///
    /// # Arguments
    ///
    /// * `vpn` - Virtual Page Number.
    /// * `ppn` - Physical Page Number.
    /// * `pte` - Raw Page Table Entry (used to extract permissions).
    /// * `asid` - Address Space Identifier from SATP[59:44].
    pub fn insert(&mut self, vpn: u64, ppn: u64, pte: u64, asid: u16) {
        let r = (pte >> 1) & 1 != 0;
        let w = (pte >> 2) & 1 != 0;
        let x = (pte >> 3) & 1 != 0;
        let u = (pte >> 4) & 1 != 0;
        let global = (pte >> 5) & 1 != 0;
        let d = (pte >> 7) & 1 != 0;

        let idx = (vpn as usize) & self.mask;

        self.entries[idx] = TlbEntry {
            vpn,
            ppn,
            valid: true,
            r,
            w,
            x,
            u,
            d,
            asid,
            global,
        };
    }

    /// Invalidates a single TLB entry by VPN (used for dirty-bit re-walk).
    pub fn invalidate(&mut self, vpn: u64) {
        let idx = (vpn as usize) & self.mask;
        if self.entries[idx].valid && self.entries[idx].vpn == vpn {
            self.entries[idx].valid = false;
        }
    }

    /// Flushes all entries from the TLB.
    ///
    /// Called when SFENCE.VMA has rs1=x0 and rs2=x0.
    pub fn flush(&mut self) {
        for e in &mut self.entries {
            e.valid = false;
        }
    }

    /// Flushes TLB entries matching a specific virtual address.
    ///
    /// Called when SFENCE.VMA has rs1!=x0 and rs2=x0.
    /// Invalidates entries whose VPN matches `vpn`, regardless of ASID.
    pub fn flush_vaddr(&mut self, vpn: u64) {
        let idx = (vpn as usize) & self.mask;
        if self.entries[idx].valid && self.entries[idx].vpn == vpn {
            self.entries[idx].valid = false;
        }
    }

    /// Flushes TLB entries matching a specific ASID.
    ///
    /// Called when SFENCE.VMA has rs1=x0 and rs2!=x0.
    /// Invalidates all non-global entries with the given ASID.
    pub fn flush_asid(&mut self, asid: u16) {
        for e in &mut self.entries {
            if e.valid && !e.global && e.asid == asid {
                e.valid = false;
            }
        }
    }

    /// Flushes TLB entries matching both a virtual address and ASID.
    ///
    /// Called when SFENCE.VMA has rs1!=x0 and rs2!=x0.
    /// Invalidates the entry at `vpn` only if it is non-global and has the given ASID.
    pub fn flush_vaddr_asid(&mut self, vpn: u64, asid: u16) {
        let idx = (vpn as usize) & self.mask;
        let e = &mut self.entries[idx];
        if e.valid && e.vpn == vpn && !e.global && e.asid == asid {
            e.valid = false;
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// L2 TLB — shared, set-associative
// ════════════════════════════════════════════════════════════════════════

/// Shared L2 TLB sitting between the per-access-type L1 TLBs and the
/// hardware page table walker. 4-way set-associative with LRU replacement.
pub struct L2Tlb {
    /// Flat array of entries: `sets * ways` elements, laid out
    /// `[set0_way0, set0_way1, …, set0_wayN, set1_way0, …]`.
    entries: Vec<TlbEntry>,
    /// Associativity (ways per set).
    ways: usize,
    /// Mask for set indexing (`num_sets - 1`).
    set_mask: usize,
    /// Per-set LRU counters. Each element is a small array of way ages
    /// (lower = more recently used). Stored flat: `[set0_way0_age, set0_way1_age, …]`.
    lru: Vec<u8>,
    /// Access latency in cycles for an L2 TLB hit.
    pub latency: u64,
}

impl L2Tlb {
    /// Creates a new L2 TLB.
    ///
    /// * `total_entries` – total capacity (rounded up to a multiple of `ways`).
    /// * `ways` – set associativity (e.g. 4).
    /// * `latency` – cycles charged on an L2 TLB hit.
    pub fn new(total_entries: usize, ways: usize, latency: u64) -> Self {
        let safe_ways = if ways == 0 { 4 } else { ways };
        let sets_raw = total_entries / safe_ways;
        let num_sets = if sets_raw.is_power_of_two() {
            sets_raw
        } else {
            sets_raw.next_power_of_two()
        }
        .max(1);
        let capacity = num_sets * safe_ways;

        Self {
            entries: vec![TlbEntry::default(); capacity],
            ways: safe_ways,
            set_mask: num_sets - 1,
            lru: vec![0u8; capacity],
            latency,
        }
    }

    /// Looks up a VPN in the L2 TLB.
    ///
    /// Returns `Some((ppn, pte_bits, asid))` on hit so the caller can
    /// promote the entry into the L1 TLB. The `pte_bits` value is a
    /// reconstructed raw PTE suitable for `Tlb::insert`.
    pub fn lookup(&mut self, vpn: u64, asid: u16) -> Option<(u64, u64, u16)> {
        let set = (vpn as usize) & self.set_mask;
        let base = set * self.ways;

        for w in 0..self.ways {
            let e = &self.entries[base + w];
            if e.valid && e.vpn == vpn && (e.global || e.asid == asid) {
                let ppn = e.ppn;
                let entry_asid = e.asid;
                let pte_bits = Self::reconstruct_pte(e);
                self.touch_lru(set, w);
                return Some((ppn, pte_bits, entry_asid));
            }
        }
        None
    }

    /// Inserts an entry, evicting the LRU way if the set is full.
    pub fn insert(&mut self, vpn: u64, ppn: u64, pte: u64, asid: u16) {
        let set = (vpn as usize) & self.set_mask;
        let base = set * self.ways;

        // Check for an existing entry with the same VPN (update in place).
        for w in 0..self.ways {
            let e = &self.entries[base + w];
            if e.valid && e.vpn == vpn && (e.global || e.asid == asid) {
                self.write_entry(base + w, vpn, ppn, pte, asid);
                self.touch_lru(set, w);
                return;
            }
        }

        // Find an invalid way first.
        for w in 0..self.ways {
            if !self.entries[base + w].valid {
                self.write_entry(base + w, vpn, ppn, pte, asid);
                self.touch_lru(set, w);
                return;
            }
        }

        // Evict LRU way (highest age value).
        let victim = self.lru_victim(set);
        self.write_entry(base + victim, vpn, ppn, pte, asid);
        self.touch_lru(set, victim);
    }

    /// Flushes all entries.
    pub fn flush(&mut self) {
        for e in &mut self.entries {
            e.valid = false;
        }
    }

    /// Flushes entries matching a specific virtual address.
    pub fn flush_vaddr(&mut self, vpn: u64) {
        let set = (vpn as usize) & self.set_mask;
        let base = set * self.ways;
        for w in 0..self.ways {
            let e = &mut self.entries[base + w];
            if e.valid && e.vpn == vpn {
                e.valid = false;
            }
        }
    }

    /// Flushes non-global entries matching a specific ASID.
    pub fn flush_asid(&mut self, asid: u16) {
        for e in &mut self.entries {
            if e.valid && !e.global && e.asid == asid {
                e.valid = false;
            }
        }
    }

    /// Flushes entries matching both a virtual address and ASID.
    pub fn flush_vaddr_asid(&mut self, vpn: u64, asid: u16) {
        let set = (vpn as usize) & self.set_mask;
        let base = set * self.ways;
        for w in 0..self.ways {
            let e = &mut self.entries[base + w];
            if e.valid && e.vpn == vpn && !e.global && e.asid == asid {
                e.valid = false;
            }
        }
    }

    // ── internal helpers ──────────────────────────────────────────────

    fn write_entry(&mut self, idx: usize, vpn: u64, ppn: u64, pte: u64, asid: u16) {
        self.entries[idx] = TlbEntry {
            vpn,
            ppn,
            valid: true,
            r: (pte >> 1) & 1 != 0,
            w: (pte >> 2) & 1 != 0,
            x: (pte >> 3) & 1 != 0,
            u: (pte >> 4) & 1 != 0,
            global: (pte >> 5) & 1 != 0,
            d: (pte >> 7) & 1 != 0,
            asid,
        };
    }

    /// Reconstruct a raw PTE value from a `TlbEntry` so it can be
    /// passed to `Tlb::insert` when promoting from L2 to L1.
    fn reconstruct_pte(e: &TlbEntry) -> u64 {
        let mut pte: u64 = 1; // V bit
        if e.r {
            pte |= 1 << 1;
        }
        if e.w {
            pte |= 1 << 2;
        }
        if e.x {
            pte |= 1 << 3;
        }
        if e.u {
            pte |= 1 << 4;
        }
        if e.global {
            pte |= 1 << 5;
        }
        if e.d {
            pte |= 1 << 7;
        }
        pte
    }

    /// Mark way `w` as most-recently-used in set `set`.
    fn touch_lru(&mut self, set: usize, way: usize) {
        let base = set * self.ways;
        let old_age = self.lru[base + way];
        for w in 0..self.ways {
            if self.lru[base + w] < old_age {
                self.lru[base + w] += 1;
            }
        }
        self.lru[base + way] = 0;
    }

    /// Returns the way index of the LRU victim in `set`.
    fn lru_victim(&self, set: usize) -> usize {
        let base = set * self.ways;
        let mut max_age = 0u8;
        let mut victim = 0;
        for w in 0..self.ways {
            if self.lru[base + w] > max_age {
                max_age = self.lru[base + w];
                victim = w;
            }
        }
        victim
    }
}

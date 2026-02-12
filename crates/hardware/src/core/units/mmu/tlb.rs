//! Translation Lookaside Buffer (TLB).
//!
//! A fully associative cache for page table entries. It stores the mapping
//! between Virtual Page Numbers (VPN) and Physical Page Numbers (PPN), along
//! with permission bits (R/W/X/U) to speed up address translation.

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
    ///
    /// # Returns
    ///
    /// `Some((ppn, r, w, x, u))` if found, otherwise `None`.
    ///
    /// # Panics
    ///
    /// This function will not panic. The unsafe array access is guaranteed safe because:
    /// - `idx = vpn & self.mask` where `mask = size - 1` (size is power of 2)
    /// - This ensures `idx` is always `< size` and within bounds of `entries`
    #[inline(always)]
    pub fn lookup(&self, vpn: u64) -> Option<(u64, bool, bool, bool, bool)> {
        let idx = (vpn as usize) & self.mask;

        // SAFETY: idx is guaranteed to be < entries.len() by the mask operation above.
        // The mask is constructed as (size - 1) where size is the length of entries,
        // ensuring idx is always a valid index.
        let entry = unsafe { self.entries.get_unchecked(idx) };

        if entry.valid && entry.vpn == vpn {
            return Some((entry.ppn, entry.r, entry.w, entry.x, entry.u));
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
    pub fn insert(&mut self, vpn: u64, ppn: u64, pte: u64) {
        let r = (pte >> 1) & 1 != 0;
        let w = (pte >> 2) & 1 != 0;
        let x = (pte >> 3) & 1 != 0;
        let u = (pte >> 4) & 1 != 0;

        let idx = (vpn as usize) & self.mask;

        self.entries[idx] = TlbEntry {
            vpn,
            ppn,
            valid: true,
            r,
            w,
            x,
            u,
        };
    }

    /// Flushes all entries from the TLB.
    ///
    /// Called on `SFENCE.VMA` instructions or SATP writes.
    pub fn flush(&mut self) {
        for e in &mut self.entries {
            e.valid = false;
        }
    }
}

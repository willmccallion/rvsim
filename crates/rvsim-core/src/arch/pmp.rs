//! Physical Memory Protection (PMP).
//!
//! This module implements RISC-V Physical Memory Protection (spec §3.7),
//! which restricts physical memory access based on the current privilege mode
//! and a set of PMP configuration registers (`pmpcfg0` and `pmpcfg2` on
//! RV64) and address registers (`pmpaddr0`–`pmpaddr15`).
//!
//! PMP supports three address-matching modes:
//! - **TOR** (Top of Range): region is `[pmpaddr[i-1], pmpaddr[i])`.
//! - **NA4**: Naturally aligned 4-byte region.
//! - **NAPOT**: Naturally aligned power-of-two region.

use crate::common::{AccessType, PhysAddr};
use crate::isa::csr::{CsrAddr, PMPADDR0, PMPADDR15};
use crate::isa::privileged::PrivilegeMode;

/// Maximum number of PMP entries (RISC-V spec allows up to 64).
pub const PMP_COUNT: usize = 16;

/// The entries each RV64 `pmpcfg` register configures, a byte each.
const ENTRIES_PER_CFG_CSR: usize = 8;

/// The number of a PMP entry the unit has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PmpIdx(usize);

impl PmpIdx {
    /// Entry `n`, if the unit has that many.
    pub const fn new(n: usize) -> Option<Self> {
        if n < PMP_COUNT { Some(Self(n)) } else { None }
    }

    /// The entry whose address register `addr` is, if it is a `pmpaddr`.
    pub const fn of_pmpaddr(addr: CsrAddr) -> Option<Self> {
        if addr.is_within(PMPADDR0, PMPADDR15) {
            Self::new((addr.as_u32() - PMPADDR0.as_u32()) as usize)
        } else {
            None
        }
    }
}

/// A PMP configuration register: on RV64, `pmpcfg0` holds entries 0–7 and
/// `pmpcfg2` entries 8–15.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PmpCfgCsr {
    /// `pmpcfg0`.
    Pmpcfg0,
    /// `pmpcfg2`.
    Pmpcfg2,
}

/// PMP address-matching mode field (bits 4:3 of pmpcfg).
const A_SHIFT: u8 = 3;
const A_MASK: u8 = 0x3;

/// The `pmpaddr` bits RV64 implements: physical address bits 55:2. Bits
/// 63:54 read as zero.
const PMPADDR_MASK: u64 = (1 << 54) - 1;

/// PMP configuration permission bits.
const PMP_R: u8 = 1 << 0;
const PMP_W: u8 = 1 << 1;
const PMP_X: u8 = 1 << 2;
const PMP_L: u8 = 1 << 7;

/// Address matching mode extracted from pmpcfg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PmpAddrMatch {
    /// Disabled — entry is off.
    Off = 0,
    /// Top of Range — region is `[pmpaddr[i-1], pmpaddr[i])`.
    Tor = 1,
    /// Naturally aligned 4-byte region.
    Na4 = 2,
    /// Naturally aligned power-of-two region.
    Napot = 3,
}

impl PmpAddrMatch {
    /// Decode from the 2-bit A field in a pmpcfg byte.
    pub const fn from_bits(bits: u8) -> Self {
        match bits & A_MASK {
            0 => Self::Off,
            1 => Self::Tor,
            2 => Self::Na4,
            _ => Self::Napot,
        }
    }
}

/// Result of a PMP permission check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PmpResult {
    /// Access is permitted.
    Allow,
    /// Access is denied.
    Deny,
    /// No PMP entry matched (default policy applies).
    NoMatch,
}

/// Decoded PMP entry with precomputed range.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PmpEntry {
    /// Raw configuration byte from pmpcfg.
    pub cfg: u8,
    /// Raw pmpaddr register value (shifted address, not byte address).
    pub addr: u64,
}

impl PmpEntry {
    /// An entry that matches nothing, as at reset.
    const OFF: Self = Self { cfg: 0, addr: 0 };

    /// Writes the configuration byte, unless the entry is locked.
    const fn set_cfg(&mut self, cfg: u8) {
        if !self.is_locked() {
            self.cfg = cfg;
        }
    }

    /// Returns the address-matching mode.
    pub const fn match_mode(&self) -> PmpAddrMatch {
        PmpAddrMatch::from_bits((self.cfg >> A_SHIFT) & A_MASK)
    }

    /// Returns true if the R (read) permission bit is set.
    pub const fn is_readable(&self) -> bool {
        self.cfg & PMP_R != 0
    }

    /// Returns true if the W (write) permission bit is set.
    pub const fn is_writable(&self) -> bool {
        self.cfg & PMP_W != 0
    }

    /// Returns true if the X (execute) permission bit is set.
    pub const fn is_executable(&self) -> bool {
        self.cfg & PMP_X != 0
    }

    /// Returns true if the L (lock) bit is set.
    pub const fn is_locked(&self) -> bool {
        self.cfg & PMP_L != 0
    }
}

/// Physical Memory Protection unit.
///
/// Maintains the PMP configuration and address registers and provides
/// a `check` method that determines whether an access at a given
/// physical address is permitted.
#[derive(Debug)]
pub struct Pmp {
    entries: [PmpEntry; PMP_COUNT],
}

impl Default for Pmp {
    fn default() -> Self {
        Self::new()
    }
}

impl Pmp {
    /// Creates a new PMP unit with all entries disabled.
    pub const fn new() -> Self {
        Self { entries: [PmpEntry::OFF; PMP_COUNT] }
    }

    /// Returns a reference to the entries slice for inspection.
    pub const fn entries(&self) -> &[PmpEntry] {
        &self.entries
    }

    /// Replaces every entry, locked or not, as restoring a checkpoint does.
    pub fn restore(&mut self, entries: &[PmpEntry]) {
        for (entry, saved) in self.entries.iter_mut().zip(entries) {
            *entry = saved.clone();
        }
    }

    /// Sets the configuration byte for entry `idx`, unless it is locked.
    pub const fn set_cfg(&mut self, idx: PmpIdx, cfg: u8) {
        self.entries[idx.0].set_cfg(cfg);
    }

    /// Sets the address register for entry `idx`, unless it is locked.
    ///
    /// `addr` is in the pmpaddr format: physical address >> 2. Only bits
    /// 53:0 are kept.
    pub fn set_addr(&mut self, idx: PmpIdx, addr: u64) {
        if !self.addr_is_locked(idx) {
            self.entries[idx.0].addr = addr & PMPADDR_MASK;
        }
    }

    /// True when `pmpaddr[idx]` ignores writes: its own entry is locked, or
    /// the next is a locked TOR entry, whose base it holds (priv. §3.7.1).
    fn addr_is_locked(&self, idx: PmpIdx) -> bool {
        let locks_base =
            |next: &PmpEntry| next.is_locked() && next.match_mode() == PmpAddrMatch::Tor;
        self.entries[idx.0].is_locked() || self.entries.get(idx.0 + 1).is_some_and(locks_base)
    }

    /// Reads the configuration byte for entry `idx`.
    pub const fn get_cfg(&self, idx: PmpIdx) -> u8 {
        self.entries[idx.0].cfg
    }

    /// The configuration register `csr`'s value: its entries' bytes,
    /// lowest-numbered first.
    pub fn cfg_word(&self, csr: PmpCfgCsr) -> u64 {
        let bytes = self.cfg_group(csr).iter().map(|entry| entry.cfg);
        bytes.rev().fold(0, |word, cfg| (word << 8) | u64::from(cfg))
    }

    /// Writes `word` to the configuration register `csr`; locked entries
    /// keep their bytes.
    pub fn set_cfg_word(&mut self, csr: PmpCfgCsr, word: u64) {
        for (entry, cfg) in self.cfg_group_mut(csr).iter_mut().zip(word.to_le_bytes()) {
            entry.set_cfg(cfg);
        }
    }

    /// The entries `csr` configures.
    const fn cfg_group(&self, csr: PmpCfgCsr) -> &[PmpEntry] {
        let (low, high) = self.entries.split_at(ENTRIES_PER_CFG_CSR);
        match csr {
            PmpCfgCsr::Pmpcfg0 => low,
            PmpCfgCsr::Pmpcfg2 => high,
        }
    }

    /// The entries `csr` configures, to write.
    const fn cfg_group_mut(&mut self, csr: PmpCfgCsr) -> &mut [PmpEntry] {
        let (low, high) = self.entries.split_at_mut(ENTRIES_PER_CFG_CSR);
        match csr {
            PmpCfgCsr::Pmpcfg0 => low,
            PmpCfgCsr::Pmpcfg2 => high,
        }
    }

    /// Reads the address register for entry `idx`.
    pub const fn get_addr(&self, idx: PmpIdx) -> u64 {
        self.entries[idx.0].addr
    }

    /// Computes the byte-address range `[lo, hi)` for a NAPOT entry.
    ///
    /// The pmpaddr encoding for NAPOT: trailing ones determine region size.
    /// The region size is `2^(trailing_ones + 3)` bytes and the base
    /// is the address with those trailing bits cleared.
    const fn napot_range(pmpaddr: u64) -> (u64, u64) {
        let trailing = (!pmpaddr).trailing_zeros() as u64;
        let size_bits = trailing + 3;
        // Avoid overflow when size_bits >= 64 (region would cover full address space).
        if size_bits >= 64 {
            return (0, u64::MAX);
        }
        let size = 1u64 << size_bits;
        let mask = size - 1;
        let base = (pmpaddr << 2) & !mask;
        (base, base.wrapping_add(size))
    }

    /// Computes the byte-address range for an NA4 entry (exactly 4 bytes).
    const fn na4_range(pmpaddr: u64) -> (u64, u64) {
        let base = pmpaddr << 2;
        (base, base + 4)
    }

    /// Checks whether an access at `byte_addr` is permitted.
    ///
    /// # Arguments
    ///
    /// * `addr` - Physical byte address of the access.
    /// * `size` - Number of bytes being accessed.
    /// * `access` - What the access does.
    /// * `privilege` - The privilege it is made at.
    ///
    /// # Returns
    ///
    /// `PmpResult::Allow` if the access is permitted, `PmpResult::Deny`
    /// if denied, `PmpResult::NoMatch` if no entry matched.
    pub fn check(
        &self,
        addr: PhysAddr,
        size: u64,
        access: AccessType,
        privilege: PrivilegeMode,
    ) -> PmpResult {
        let byte_addr = addr.val();
        let is_machine_mode = privilege == PrivilegeMode::Machine;
        // Saturate: an access past u64::MAX can't physically fit, and
        // clamping still gives correct partial-overlap / no-match behavior.
        let access_end = byte_addr.saturating_add(size);

        for i in 0..self.entries.len() {
            let entry = &self.entries[i];
            let (lo, hi) = match entry.match_mode() {
                PmpAddrMatch::Tor => {
                    let hi = entry.addr << 2;
                    let lo = if i == 0 { 0 } else { self.entries[i - 1].addr << 2 };
                    (lo, hi)
                }
                PmpAddrMatch::Na4 => Self::na4_range(entry.addr),
                PmpAddrMatch::Napot => Self::napot_range(entry.addr),
                PmpAddrMatch::Off => continue,
            };

            // Spec §3.7.1: lowest-numbered entry matching any byte decides;
            // partial overlap denies regardless of permissions.
            let any_byte_match = byte_addr < hi && access_end > lo;
            if any_byte_match {
                let all_bytes_match = byte_addr >= lo && access_end <= hi;

                if !all_bytes_match {
                    return PmpResult::Deny;
                }

                if is_machine_mode && !entry.is_locked() {
                    return PmpResult::Allow;
                }

                let permitted = match access {
                    AccessType::Fetch => entry.is_executable(),
                    AccessType::Read => entry.is_readable(),
                    AccessType::Write => entry.is_writable(),
                };

                return if permitted { PmpResult::Allow } else { PmpResult::Deny };
            }
        }

        // M-mode bypasses PMP when no entry matches (spec §3.7.1).
        if is_machine_mode { PmpResult::Allow } else { PmpResult::NoMatch }
    }
}

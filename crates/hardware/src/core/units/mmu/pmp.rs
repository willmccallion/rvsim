//! Physical Memory Protection (PMP).
//!
//! This module implements RISC-V Physical Memory Protection (spec §3.7),
//! which restricts physical memory access based on the current privilege mode
//! and a set of PMP configuration registers (`pmpcfg0`–`pmpcfg15`) and
//! address registers (`pmpaddr0`–`pmpaddr63`).
//!
//! PMP supports three address-matching modes:
//! - **TOR** (Top of Range): region is `[pmpaddr[i-1], pmpaddr[i])`.
//! - **NA4**: Naturally aligned 4-byte region.
//! - **NAPOT**: Naturally aligned power-of-two region.

/// Maximum number of PMP entries (RISC-V spec allows up to 64).
pub const PMP_COUNT: usize = 16;

/// PMP address-matching mode field (bits 4:3 of pmpcfg).
const A_SHIFT: u8 = 3;
const A_MASK: u8 = 0x3;

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
    pub fn from_bits(bits: u8) -> Self {
        match bits & A_MASK {
            0 => Self::Off,
            1 => Self::Tor,
            2 => Self::Na4,
            3 => Self::Napot,
            _ => unreachable!(),
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
#[derive(Clone, Debug)]
pub struct PmpEntry {
    /// Raw configuration byte from pmpcfg.
    pub cfg: u8,
    /// Raw pmpaddr register value (shifted address, not byte address).
    pub addr: u64,
}

impl PmpEntry {
    /// Returns the address-matching mode.
    pub fn match_mode(&self) -> PmpAddrMatch {
        PmpAddrMatch::from_bits((self.cfg >> A_SHIFT) & A_MASK)
    }

    /// Returns true if the R (read) permission bit is set.
    pub fn is_readable(&self) -> bool {
        self.cfg & PMP_R != 0
    }

    /// Returns true if the W (write) permission bit is set.
    pub fn is_writable(&self) -> bool {
        self.cfg & PMP_W != 0
    }

    /// Returns true if the X (execute) permission bit is set.
    pub fn is_executable(&self) -> bool {
        self.cfg & PMP_X != 0
    }

    /// Returns true if the L (lock) bit is set.
    pub fn is_locked(&self) -> bool {
        self.cfg & PMP_L != 0
    }
}

/// Physical Memory Protection unit.
///
/// Maintains the PMP configuration and address registers and provides
/// a `check` method that determines whether an access at a given
/// physical address is permitted.
pub struct Pmp {
    /// PMP entries (up to `PMP_COUNT`).
    entries: Vec<PmpEntry>,
}

impl Pmp {
    /// Creates a new PMP unit with all entries disabled.
    pub fn new() -> Self {
        let entries = (0..PMP_COUNT)
            .map(|_| PmpEntry { cfg: 0, addr: 0 })
            .collect();
        Self { entries }
    }

    /// Returns a reference to the entries slice for inspection.
    pub fn entries(&self) -> &[PmpEntry] {
        &self.entries
    }

    /// Sets the configuration byte for entry `idx`.
    pub fn set_cfg(&mut self, idx: usize, cfg: u8) {
        if idx < self.entries.len() {
            // Locked entries cannot be modified.
            if self.entries[idx].cfg & PMP_L != 0 {
                return;
            }
            self.entries[idx].cfg = cfg;
        }
    }

    /// Sets the address register for entry `idx`.
    ///
    /// `addr` is in the pmpaddr format: physical address >> 2.
    pub fn set_addr(&mut self, idx: usize, addr: u64) {
        if idx < self.entries.len() {
            if self.entries[idx].cfg & PMP_L != 0 {
                return;
            }
            self.entries[idx].addr = addr;
        }
    }

    /// Reads the configuration byte for entry `idx`.
    pub fn get_cfg(&self, idx: usize) -> u8 {
        if idx < self.entries.len() {
            self.entries[idx].cfg
        } else {
            0
        }
    }

    /// Reads the address register for entry `idx`.
    pub fn get_addr(&self, idx: usize) -> u64 {
        if idx < self.entries.len() {
            self.entries[idx].addr
        } else {
            0
        }
    }

    /// Computes the byte-address range `[lo, hi)` for a NAPOT entry.
    ///
    /// The pmpaddr encoding for NAPOT: trailing ones determine region size.
    /// The region size is `2^(trailing_ones + 3)` bytes and the base
    /// is the address with those trailing bits cleared.
    fn napot_range(pmpaddr: u64) -> (u64, u64) {
        // Count trailing ones in pmpaddr
        let trailing = (!pmpaddr).trailing_zeros() as u64;
        // Region size = 2^(trailing + 3) bytes
        let size = 1u64 << (trailing + 3);
        // Mask the trailing bits + the implicit bit above them
        let mask = size - 1;
        let base = (pmpaddr << 2) & !mask;
        (base, base + size)
    }

    /// Computes the byte-address range for an NA4 entry (exactly 4 bytes).
    fn na4_range(pmpaddr: u64) -> (u64, u64) {
        let base = pmpaddr << 2;
        (base, base + 4)
    }

    /// Checks whether an access at `byte_addr` is permitted.
    ///
    /// # Arguments
    ///
    /// * `byte_addr` - Physical byte address of the access.
    /// * `size` - Number of bytes being accessed.
    /// * `is_read` - True for load operations.
    /// * `is_write` - True for store operations.
    /// * `is_exec` - True for instruction fetch.
    /// * `is_machine_mode` - True if the current privilege is M-mode.
    ///
    /// # Returns
    ///
    /// `PmpResult::Allow` if the access is permitted, `PmpResult::Deny`
    /// if denied, `PmpResult::NoMatch` if no entry matched.
    pub fn check(
        &self,
        byte_addr: u64,
        size: u64,
        is_read: bool,
        is_write: bool,
        is_exec: bool,
        is_machine_mode: bool,
    ) -> PmpResult {
        let access_end = byte_addr + size;

        for i in 0..self.entries.len() {
            let entry = &self.entries[i];
            let mode = entry.match_mode();

            if mode == PmpAddrMatch::Off {
                continue;
            }

            let (lo, hi) = match mode {
                PmpAddrMatch::Tor => {
                    let hi = entry.addr << 2;
                    let lo = if i == 0 {
                        0
                    } else {
                        self.entries[i - 1].addr << 2
                    };
                    (lo, hi)
                }
                PmpAddrMatch::Na4 => Self::na4_range(entry.addr),
                PmpAddrMatch::Napot => Self::napot_range(entry.addr),
                PmpAddrMatch::Off => continue,
            };

            // Check if the access range overlaps with this entry
            if byte_addr >= lo && access_end <= hi {
                // M-mode: if the entry is NOT locked, M-mode bypasses PMP.
                if is_machine_mode && !entry.is_locked() {
                    return PmpResult::Allow;
                }

                // Check permissions
                let permitted = (!is_read || entry.is_readable())
                    && (!is_write || entry.is_writable())
                    && (!is_exec || entry.is_executable());

                return if permitted {
                    PmpResult::Allow
                } else {
                    PmpResult::Deny
                };
            }
        }

        // No entry matched.
        // M-mode: if no entries match, M-mode has full access (spec §3.7.1).
        if is_machine_mode {
            PmpResult::Allow
        } else {
            PmpResult::NoMatch
        }
    }
}

//! Address-translation modes `satp.MODE` selects.

use serde::{Deserialize, Serialize};

use crate::common::Asid;
use crate::isa::csr::{
    SATP_ASID_MASK, SATP_ASID_SHIFT, SATP_MODE_BARE, SATP_MODE_MASK, SATP_MODE_SHIFT,
    SATP_MODE_SV39, SATP_MODE_SV48, SATP_MODE_SV57, SATP_PPN_MASK,
};

/// A `satp` value: the paging mode, the address space and the root page
/// table it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Satp(u64);

impl Satp {
    /// The register value `bits`.
    #[must_use]
    pub const fn new(bits: u64) -> Self {
        Self(bits)
    }

    /// The raw MODE field.
    #[must_use]
    pub const fn mode_field(self) -> u64 {
        (self.0 >> SATP_MODE_SHIFT) & SATP_MODE_MASK
    }

    /// The paging mode MODE names, or `None` for one not implemented.
    #[must_use]
    pub const fn paging_mode(self) -> Option<PagingMode> {
        PagingMode::from_satp_mode(self.mode_field())
    }

    /// The address-space identifier.
    #[must_use]
    pub const fn asid(self) -> Asid {
        Asid::new(((self.0 >> SATP_ASID_SHIFT) & SATP_ASID_MASK) as u16)
    }

    /// The physical page number of the root page table.
    #[must_use]
    pub const fn root_ppn(self) -> u64 {
        self.0 & SATP_PPN_MASK
    }
}

/// Supervisor address-translation modes recognised by this implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PagingMode {
    /// No translation; virtual address is the physical address.
    Bare,
    /// 3-level walk over a 39-bit virtual address.
    Sv39,
    /// 4-level walk over a 48-bit virtual address.
    Sv48,
    /// 5-level walk over a 57-bit virtual address.
    Sv57,
}

impl PagingMode {
    /// Decodes a raw `satp.MODE` value. Returns `None` for unsupported modes
    /// (Sv32 on RV64, hypervisor variants, reserved encodings).
    pub const fn from_satp_mode(mode: u64) -> Option<Self> {
        match mode {
            SATP_MODE_BARE => Some(Self::Bare),
            SATP_MODE_SV39 => Some(Self::Sv39),
            SATP_MODE_SV48 => Some(Self::Sv48),
            SATP_MODE_SV57 => Some(Self::Sv57),
            _ => None,
        }
    }

    /// Number of page-table levels to walk for this mode.
    pub const fn levels(self) -> usize {
        match self {
            Self::Bare => 0,
            Self::Sv39 => 3,
            Self::Sv48 => 4,
            Self::Sv57 => 5,
        }
    }

    /// Index of the highest meaningful VA bit. Bits strictly above this must
    /// equal this bit (sign-extension); otherwise the address is non-canonical.
    pub const fn va_top_bit(self) -> u32 {
        match self {
            Self::Bare => 63,
            Self::Sv39 => 38,
            Self::Sv48 => 47,
            Self::Sv57 => 56,
        }
    }

    /// True if this mode is no stronger than `cap`. Used by the SATP writer
    /// to optionally pin the maximum paging level the simulator accepts.
    /// Bare is weaker than every Sv mode; Sv39 < Sv48 < Sv57.
    pub const fn is_at_most(self, cap: Self) -> bool {
        self.levels() <= cap.levels()
    }
}

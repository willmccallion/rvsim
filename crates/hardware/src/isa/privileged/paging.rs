//! Address-translation modes `satp.MODE` selects.

use crate::isa::csr::{SATP_MODE_BARE, SATP_MODE_SV39, SATP_MODE_SV48, SATP_MODE_SV57};

/// Supervisor address-translation modes recognised by this implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

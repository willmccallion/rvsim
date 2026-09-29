//! Floating-point rounding modes and accrued exception flags (`fcsr`).

use std::ops::BitOr;

/// RISC-V rounding mode encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RoundingMode {
    /// Round to Nearest, ties to Even (default IEEE mode).
    Rne = 0b000,
    /// Round towards Zero.
    Rtz = 0b001,
    /// Round Down (towards −∞).
    Rdn = 0b010,
    /// Round Up (towards +∞).
    Rup = 0b011,
    /// Round to Nearest, ties to Max Magnitude.
    Rmm = 0b100,
}

impl RoundingMode {
    /// Decodes a 3-bit rounding mode field from an instruction or `fcsr.frm`.
    ///
    /// Returns `None` for reserved encodings (0b101, 0b110) and the dynamic
    /// sentinel (0b111), which must be resolved to `fcsr.frm` by the caller.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits & 0x7 {
            0b000 => Some(Self::Rne),
            0b001 => Some(Self::Rtz),
            0b010 => Some(Self::Rdn),
            0b011 => Some(Self::Rup),
            0b100 => Some(Self::Rmm),
            _ => None, // 0b101, 0b110 reserved; 0b111 = dynamic
        }
    }
}

/// Floating-point exception flags (RISC-V `fcsr.fflags`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FpFlags(u8);

impl FpFlags {
    /// No exceptions raised.
    pub const NONE: Self = Self(0);
    /// Invalid Operation.
    pub const NV: Self = Self(1 << 4);
    /// Divide by Zero.
    pub const DZ: Self = Self(1 << 3);
    /// Overflow.
    pub const OF: Self = Self(1 << 2);
    /// Underflow.
    pub const UF: Self = Self(1 << 1);
    /// Inexact.
    pub const NX: Self = Self(1 << 0);

    /// Creates `FpFlags` from a raw 5-bit value.
    pub const fn from_bits(val: u8) -> Self {
        Self(val & 0x1F)
    }

    /// Returns the raw 5-bit flag value for writing into `fcsr`.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns true if no flags are set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns true if the specified flag is set.
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl BitOr for FpFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

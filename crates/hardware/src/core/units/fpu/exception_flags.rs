//! Floating-point exception (accrued) flags.
//!
//! RISC-V defines five exception flags in `fcsr.fflags` (spec §11.2):
//!
//! | Bit | Flag | Description         |
//! |-----|------|---------------------|
//! |  4  | NV   | Invalid Operation   |
//! |  3  | DZ   | Divide by Zero      |
//! |  2  | OF   | Overflow            |
//! |  1  | UF   | Underflow           |
//! |  0  | NX   | Inexact             |
//!
//! This module will provide flag accumulation and the `execute_full` API
//! that returns both the result and the set of raised exception flags.
//!
//! **Status:** Stub — implementation pending as part of Phase 1.2 exception
//! flag verification.

use std::ops::BitOr;

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

    /// Returns the raw 5-bit flag value for writing into `fcsr`.
    pub fn bits(self) -> u8 {
        self.0
    }

    /// Returns true if no flags are set.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns true if the specified flag is set.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl BitOr for FpFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

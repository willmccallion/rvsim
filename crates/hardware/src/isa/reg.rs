//! Architectural register indices and the ABI names of the registers the
//! simulator itself refers to.

/// A 5-bit architectural register index (0–31).
///
/// Both integer (`x0`–`x31`) and floating-point (`f0`–`f31`) register
/// files use the same 5-bit index space defined by the RISC-V ISA.
///
/// # Invariant
///
/// The inner value is guaranteed to be in the range `0..=31`.
/// The [`RegIdx::new`] constructor panics in debug builds and
/// saturates in release builds if the value exceeds 31.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RegIdx(u8);

impl RegIdx {
    /// Creates a `RegIdx` from a raw `u8` value.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `idx > 31`.
    #[inline(always)]
    pub const fn new(idx: u8) -> Self {
        debug_assert!(idx <= 31, "RegIdx out of range: must be 0..=31");
        Self(idx & 0x1F)
    }

    /// Returns the index as a `usize` for use as an array subscript.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw `u8` value.
    #[inline(always)]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Returns `true` if this is the zero register (`x0`).
    ///
    /// Writes to `x0` are always discarded; reads always return 0.
    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl From<u8> for RegIdx {
    #[inline(always)]
    fn from(v: u8) -> Self {
        Self::new(v)
    }
}

impl From<usize> for RegIdx {
    /// Converts a `usize` to a `RegIdx`, masking to 5 bits.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `v > 31`.
    #[inline(always)]
    fn from(v: usize) -> Self {
        debug_assert!(v <= 31, "RegIdx from usize out of range: {v}");
        Self((v & 0x1F) as u8)
    }
}

impl From<RegIdx> for usize {
    #[inline(always)]
    fn from(r: RegIdx) -> Self {
        r.0 as Self
    }
}

impl From<RegIdx> for u8 {
    #[inline(always)]
    fn from(r: RegIdx) -> Self {
        r.0
    }
}

impl std::fmt::Display for RegIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "x{}", self.0)
    }
}

/// Register x0 (zero register, always zero).
pub const REG_ZERO: RegIdx = RegIdx::new(0);
/// Register x1 (return address, ra).
pub const REG_RA: RegIdx = RegIdx::new(1);
/// Register x2 (stack pointer, sp).
pub const REG_SP: RegIdx = RegIdx::new(2);
/// Register x5 (alternate link register, t0).
pub const REG_T0: RegIdx = RegIdx::new(5);
/// Register x10 (first argument/return value, a0).
pub const REG_A0: RegIdx = RegIdx::new(10);
/// Register x11 (second argument, a1).
pub const REG_A1: RegIdx = RegIdx::new(11);
/// Register x12 (third argument, a2).
pub const REG_A2: RegIdx = RegIdx::new(12);
/// Register x17 (system call number, a7).
pub const REG_A7: RegIdx = RegIdx::new(17);

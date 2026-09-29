//! Hart, core and instruction-sequence identifier newtypes.
//!
//! `HartId` is a RISC-V hardware thread (what `mhartid` reads); `CoreId` is a
//! physical core that hosts one or more harts via SMT.

/// A RISC-V hardware thread identifier.
///
/// Equal to the value the `mhartid` CSR reports for the hart. Globally unique
/// across the simulated `SoC`: with `harts_per_core > 1`, sibling SMT threads
/// have distinct `HartId`s within the same `CoreId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HartId(u32);

impl HartId {
    /// Creates a new `HartId` from a raw 32-bit value.
    #[inline(always)]
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw 32-bit value.
    #[inline(always)]
    pub const fn val(self) -> u32 {
        self.0
    }

    /// Returns the value as a `usize` for use as a vector index.
    #[inline(always)]
    pub const fn as_index(self) -> usize {
        self.0 as usize
    }
}

/// A physical-core identifier within the simulated `SoC`.
///
/// Used to address `SystemState::cores` and to disambiguate the source
/// of coherence requests in the fabric. Distinct from `HartId`: a core hosts
/// `harts_per_core` hardware threads, each with its own `HartId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CoreId(u32);

impl CoreId {
    /// Creates a new `CoreId` from a raw 32-bit value.
    #[inline(always)]
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw 32-bit value.
    #[inline(always)]
    pub const fn val(self) -> u32 {
        self.0
    }

    /// Returns the value as a `usize` for use as a vector index.
    #[inline(always)]
    pub const fn as_index(self) -> usize {
        self.0 as usize
    }
}

/// An instruction's place in fetch order, gem5's `InstSeqNum`.
///
/// Fetch numbers every instruction it forms, counting up and never reusing
/// a number, so an older instruction always has the smaller one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct InstSeq(u64);

impl InstSeq {
    /// Creates an `InstSeq` from a raw count.
    #[inline(always)]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Returns the raw count.
    #[inline(always)]
    pub const fn val(self) -> u64 {
        self.0
    }

    /// The number fetch gives the instruction after this one.
    #[inline(always)]
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

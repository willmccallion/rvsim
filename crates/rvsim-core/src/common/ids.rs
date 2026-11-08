//! Hart, core and instruction-sequence identifier newtypes.
//!
//! `HartId` is a RISC-V hardware thread (what `mhartid` reads); `CoreId` is a
//! physical core that hosts one or more harts via SMT.

use serde::{Deserialize, Serialize};

macro_rules! impl_id_new {
    ($id:ident, $raw:ty) => {
        impl $id {
            /// Creates a new identifier from a raw value.
            #[inline(always)]
            pub const fn new(val: $raw) -> Self {
                Self(val)
            }
        }
    };
}

macro_rules! impl_id_val {
    ($id:ident, $raw:ty) => {
        impl $id {
            /// Returns the raw value.
            #[inline(always)]
            pub const fn val(self) -> $raw {
                self.0
            }
        }
    };
}

macro_rules! impl_id_index {
    ($id:ident) => {
        impl $id {
            /// Returns the value as a `usize` for use as a vector index.
            #[inline(always)]
            pub const fn as_index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

macro_rules! impl_id_from_index {
    ($id:ident, $raw:ty) => {
        impl $id {
            /// The identifier of entry `index` in the collection it indexes,
            /// whose length always fits the raw type.
            #[inline(always)]
            pub fn from_index(index: usize) -> Self {
                Self(<$raw>::try_from(index).unwrap_or(<$raw>::MAX))
            }
        }
    };
}

pub(crate) use {impl_id_from_index, impl_id_index, impl_id_new, impl_id_val};

/// A RISC-V hardware thread identifier.
///
/// Equal to the value the `mhartid` CSR reports for the hart. Globally unique
/// across the simulated `SoC`: with `harts_per_core > 1`, sibling SMT threads
/// have distinct `HartId`s within the same `CoreId`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct HartId(u32);

/// A physical-core identifier within the simulated `SoC`.
///
/// Used to address `SystemState::cores` and to disambiguate the source
/// of coherence requests in the fabric. Distinct from `HartId`: a core hosts
/// `harts_per_core` hardware threads, each with its own `HartId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CoreId(u32);

/// An instruction's place in fetch order, gem5's `InstSeqNum`.
///
/// Fetch numbers every instruction it forms, counting up and never reusing
/// a number, so an older instruction always has the smaller one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct InstSeq(u64);

/// A fetch group's place in fetch order, which the fetch reorder buffer
/// releases groups in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FetchSeq(u64);

impl InstSeq {
    /// The number fetch gives the instruction after this one.
    #[inline(always)]
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl FetchSeq {
    /// The number fetch gives the group after this one.
    #[inline(always)]
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl_id_new!(HartId, u32);
impl_id_val!(HartId, u32);
impl_id_index!(HartId);
impl_id_from_index!(HartId, u32);
impl_id_new!(CoreId, u32);
impl_id_val!(CoreId, u32);
impl_id_index!(CoreId);
impl_id_from_index!(CoreId, u32);
impl_id_new!(InstSeq, u64);
impl_id_val!(InstSeq, u64);
impl_id_val!(FetchSeq, u64);

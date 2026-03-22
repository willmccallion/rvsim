//! Newtypes for the store-set memory dependence predictor.

/// Store Set ID — indexes into the LFST.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StoreSetId(pub u16);

/// Index into the SSIT table. Derived from PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SsitIndex(pub u32);

impl SsitIndex {
    /// Compute the SSIT index for a given PC and table size.
    #[inline]
    pub const fn from_pc(pc: u64, table_size: usize) -> Self {
        Self(((pc >> 2) as u32) % (table_size as u32))
    }
}

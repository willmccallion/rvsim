//! Vector register indices, effective element widths and register
//! groupings, and element indexing.

use crate::isa::rvv::{LmulGroup, Sew, Vlmul};

/// Vector register index (0–31). NOT interchangeable with `RegIdx` (GPR/FPR).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct VRegIdx(u8);

impl VRegIdx {
    /// Creates a `VRegIdx` from a raw `u8`.
    ///
    /// # Panics
    ///
    /// Panics if `val >= 32`.
    #[inline(always)]
    pub const fn new(val: u8) -> Self {
        assert!(val < 32, "vector register index out of range");
        Self(val)
    }

    /// Returns the raw index.
    #[inline(always)]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Returns the index as a `usize` for array subscript.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns `true` if this is v0 (mask register).
    #[inline(always)]
    pub const fn is_v0(self) -> bool {
        self.0 == 0
    }

    /// Check if this register is a valid base for an LMUL group.
    #[inline(always)]
    pub const fn is_aligned(self, group: LmulGroup) -> bool {
        self.0.is_multiple_of(group.regs())
    }
}

impl std::fmt::Display for VRegIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Effective Element Width for loads/stores (can differ from SEW).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Eew(Sew);

impl Eew {
    /// Create from a `Sew` value.
    #[inline(always)]
    pub const fn new(sew: Sew) -> Self {
        Self(sew)
    }

    /// Returns the underlying `Sew`.
    #[inline(always)]
    pub const fn sew(self) -> Sew {
        self.0
    }

    /// Width in bits.
    #[inline(always)]
    pub const fn bits(self) -> usize {
        self.0.bits()
    }

    /// Width in bytes.
    #[inline(always)]
    pub const fn bytes(self) -> usize {
        self.0.bytes()
    }
}

/// Element index within a vector register (group).
/// Range: 0..VLMAX. NOT interchangeable with plain `usize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ElemIdx(usize);

impl ElemIdx {
    /// Create a new element index.
    #[inline(always)]
    pub const fn new(val: usize) -> Self {
        Self(val)
    }

    /// Returns the index as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Segment field count (nf). Range 1..=8 (encoded as 0..=7 in instruction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Nf(u8);

impl Nf {
    /// Create from the 3-bit encoded value (0..=7 → fields 1..=8).
    #[inline(always)]
    pub const fn from_encoding(enc: u8) -> Self {
        Self((enc & 0x7) + 1)
    }

    /// Returns the actual field count (1..=8).
    #[inline(always)]
    pub const fn fields(self) -> u8 {
        self.0
    }

    /// Returns the field count as usize.
    #[inline(always)]
    pub const fn fields_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns true if this is a non-segment operation (nf=1).
    #[inline(always)]
    pub const fn is_single(self) -> bool {
        self.0 == 1
    }
}

/// Effective LMUL for a vector operand. Determines how many physical
/// registers one segment field spans. Used for segment load/store field spacing
/// (RVV 1.0 §7.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Emul(u8);

impl Emul {
    /// Compute EMUL = max(1, (EEW / SEW) * LMUL).
    ///
    /// For unit-stride and strided loads where EEW == SEW, this equals the
    /// LMUL group size. For indexed loads the EEW of indices may differ from
    /// the data SEW, producing a different EMUL.
    pub const fn compute(eew: Sew, sew: Sew, lmul: Vlmul) -> Self {
        let (lnum, lden) = lmul.as_fraction();
        let emul_num = eew.bits() * lnum;
        let emul_den = sew.bits() * lden;
        let emul = if emul_num >= emul_den { emul_num / emul_den } else { 1 };
        Self(emul as u8)
    }

    /// Number of consecutive registers per segment field.
    #[inline(always)]
    pub const fn regs(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_vregidx_range() {
        let v0 = VRegIdx::new(0);
        assert!(v0.is_v0());
        assert_eq!(v0.as_u8(), 0);

        let v31 = VRegIdx::new(31);
        assert_eq!(v31.as_u8(), 31);
        assert!(!v31.is_v0());
    }

    #[test]
    #[should_panic(expected = "vector register index out of range")]
    fn test_vregidx_out_of_range() {
        let _ = VRegIdx::new(32);
    }

    #[test]
    fn test_vregidx_alignment() {
        let v0 = VRegIdx::new(0);
        let v1 = VRegIdx::new(1);
        let v4 = VRegIdx::new(4);
        let group4 = Vlmul::M4.group_regs();

        assert!(v0.is_aligned(group4));
        assert!(!v1.is_aligned(group4));
        assert!(v4.is_aligned(group4));
    }

    #[test]
    fn test_nf() {
        let nf = Nf::from_encoding(0);
        assert_eq!(nf.fields(), 1);
        assert!(nf.is_single());

        let nf7 = Nf::from_encoding(7);
        assert_eq!(nf7.fields(), 8);
        assert!(!nf7.is_single());
    }
}

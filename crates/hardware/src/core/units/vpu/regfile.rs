//! VectorRegFile trait: abstracting vector register access for O3 pipeline integration.
//!
//! The VPU execution modules (alu, fpu, reduction, mask, permute) are generic over
//! this trait, allowing them to operate on either the architectural VPR (in-order
//! backend) or a VecPrfView (O3 backend with physical register renaming).

use crate::core::units::vpu::types::{ElemIdx, Sew, VRegIdx, Vlen};

/// Trait abstracting element-level access to vector registers.
///
/// Implemented by:
/// - [`Vpr`](crate::core::arch::vpr::Vpr) — architectural VRF (in-order backend)
/// - [`VecPrfView`](crate::core::pipeline::vec_prf::VecPrfView) — O3 physical VRF with renaming
pub trait VectorRegFile {
    /// Read a single element from a vector register, zero-extended to u64.
    fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64;

    /// Write a single element to a vector register (truncated to SEW bits).
    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64);

    /// Read a single mask bit from a vector register.
    fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool;

    /// Write a single mask bit in a vector register.
    fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool);

    /// Copy one register to another.
    fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx);

    /// Returns the configured VLEN.
    fn vlen(&self) -> Vlen;
}

// Implement for the architectural VPR.
impl VectorRegFile for crate::core::arch::vpr::Vpr {
    #[inline]
    fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        self.read_element(vreg, index, sew)
    }

    #[inline]
    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        self.write_element(vreg, index, sew, val);
    }

    #[inline]
    fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool {
        self.read_mask_bit(vreg, index)
    }

    #[inline]
    fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool) {
        self.write_mask_bit(vreg, index, val);
    }

    #[inline]
    fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx) {
        self.copy_reg(dst, src);
    }

    #[inline]
    fn vlen(&self) -> Vlen {
        self.vlen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::arch::vpr::Vpr;

    /// Verify that the trait methods on Vpr match the direct methods.
    #[test]
    fn test_vpr_trait_impl() {
        let mut vpr = Vpr::new(Vlen::new_unchecked(128));
        let v1 = VRegIdx::new(1);

        // Write via trait
        VectorRegFile::write_element(&mut vpr, v1, ElemIdx::new(0), Sew::E32, 0xDEAD_BEEF);
        assert_eq!(VectorRegFile::read_element(&vpr, v1, ElemIdx::new(0), Sew::E32), 0xDEAD_BEEF);

        // Mask bits via trait
        let v0 = VRegIdx::new(0);
        VectorRegFile::write_mask_bit(&mut vpr, v0, ElemIdx::new(3), true);
        assert!(VectorRegFile::read_mask_bit(&vpr, v0, ElemIdx::new(3)));
        assert!(!VectorRegFile::read_mask_bit(&vpr, v0, ElemIdx::new(4)));

        // copy_reg via trait
        let v2 = VRegIdx::new(2);
        VectorRegFile::copy_reg(&mut vpr, v2, v1);
        assert_eq!(VectorRegFile::read_element(&vpr, v2, ElemIdx::new(0), Sew::E32), 0xDEAD_BEEF);

        // vlen via trait
        assert_eq!(VectorRegFile::vlen(&vpr).bits(), 128);
    }
}

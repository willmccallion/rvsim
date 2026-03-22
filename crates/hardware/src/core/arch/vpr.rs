//! Architectural Vector Register File (VPR).
//!
//! Provides 32 × VLEN-bit vector registers with element-wise access at all SEW
//! values and register group operations for LMUL > 1.

use crate::core::units::vpu::types::{ElemIdx, LmulGroup, Sew, VRegIdx, Vlen};

/// Architectural vector register file: 32 registers, each VLEN bits wide.
pub struct Vpr {
    /// Contiguous storage: 32 * `vlen.bytes()` bytes.
    data: Vec<u8>,
    /// The configured VLEN.
    vlen: Vlen,
}

impl Vpr {
    /// Create a new VPR with all registers zeroed.
    pub fn new(vlen: Vlen) -> Self {
        Self { data: vec![0u8; 32 * vlen.bytes()], vlen }
    }

    /// Returns the configured VLEN.
    #[inline]
    pub const fn vlen(&self) -> Vlen {
        self.vlen
    }

    /// Byte offset for a given register and element.
    #[inline]
    const fn offset(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> usize {
        vreg.as_usize() * self.vlen.bytes() + index.as_usize() * sew.bytes()
    }

    /// Read a single element from a vector register.
    ///
    /// Returns the element zero-extended to u64.
    pub fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        let off = self.offset(vreg, index, sew);
        let bytes = sew.bytes();
        debug_assert!(
            off + bytes <= self.data.len(),
            "VPR read out of bounds: vreg={}, idx={}, sew={:?}",
            vreg.as_u8(),
            index.as_usize(),
            sew
        );
        let mut val = 0u64;
        for i in 0..bytes {
            val |= (self.data[off + i] as u64) << (i * 8);
        }
        val
    }

    /// Write a single element to a vector register.
    ///
    /// The value is truncated to SEW bits.
    pub fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        let off = self.offset(vreg, index, sew);
        let bytes = sew.bytes();
        debug_assert!(
            off + bytes <= self.data.len(),
            "VPR write out of bounds: vreg={}, idx={}, sew={:?}",
            vreg.as_u8(),
            index.as_usize(),
            sew
        );
        for i in 0..bytes {
            self.data[off + i] = (val >> (i * 8)) as u8;
        }
    }

    /// Read the raw bytes of a single vector register.
    pub fn read_bytes(&self, vreg: VRegIdx) -> &[u8] {
        let start = vreg.as_usize() * self.vlen.bytes();
        &self.data[start..start + self.vlen.bytes()]
    }

    /// Write raw bytes to a single vector register.
    ///
    /// # Panics
    ///
    /// Panics if `data.len() != vlen.bytes()`.
    pub fn write_bytes(&mut self, vreg: VRegIdx, data: &[u8]) {
        assert_eq!(data.len(), self.vlen.bytes(), "write_bytes: length mismatch");
        let start = vreg.as_usize() * self.vlen.bytes();
        self.data[start..start + self.vlen.bytes()].copy_from_slice(data);
    }

    /// Read a single mask bit from a vector register.
    ///
    /// Mask bit `i` is stored as bit `i % 8` of byte `i / 8` within the register.
    pub fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool {
        let byte_off = vreg.as_usize() * self.vlen.bytes() + index.as_usize() / 8;
        let bit_off = index.as_usize() % 8;
        debug_assert!(byte_off < self.data.len(), "mask bit read out of bounds");
        (self.data[byte_off] >> bit_off) & 1 != 0
    }

    /// Write a single mask bit in a vector register.
    pub fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool) {
        let byte_off = vreg.as_usize() * self.vlen.bytes() + index.as_usize() / 8;
        let bit_off = index.as_usize() % 8;
        debug_assert!(byte_off < self.data.len(), "mask bit write out of bounds");
        if val {
            self.data[byte_off] |= 1 << bit_off;
        } else {
            self.data[byte_off] &= !(1 << bit_off);
        }
    }

    /// Read the raw bytes of a register group (LMUL consecutive registers).
    ///
    /// # Panics
    ///
    /// Panics if `base` is not aligned to the group size.
    pub fn read_group(&self, base: VRegIdx, group: LmulGroup) -> &[u8] {
        assert!(
            base.is_aligned(group),
            "register group base {} not aligned to LMUL group size {}",
            base.as_u8(),
            group.regs()
        );
        let start = base.as_usize() * self.vlen.bytes();
        let len = group.regs_usize() * self.vlen.bytes();
        &self.data[start..start + len]
    }

    /// Write raw bytes to a register group (LMUL consecutive registers).
    ///
    /// # Panics
    ///
    /// Panics if `base` is not aligned or `data.len()` doesn't match.
    pub fn write_group(&mut self, base: VRegIdx, group: LmulGroup, data: &[u8]) {
        assert!(
            base.is_aligned(group),
            "register group base {} not aligned to LMUL group size {}",
            base.as_u8(),
            group.regs()
        );
        let len = group.regs_usize() * self.vlen.bytes();
        assert_eq!(data.len(), len, "write_group: length mismatch");
        let start = base.as_usize() * self.vlen.bytes();
        self.data[start..start + len].copy_from_slice(data);
    }

    /// Copy one register to another.
    pub fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx) {
        if dst == src {
            return;
        }
        let vlen_bytes = self.vlen.bytes();
        let src_start = src.as_usize() * vlen_bytes;
        let dst_start = dst.as_usize() * vlen_bytes;
        // Use copy_within to handle the case where src and dst are in the same allocation.
        self.data.copy_within(src_start..src_start + vlen_bytes, dst_start);
    }

    /// Zero a single register.
    pub fn zero_reg(&mut self, vreg: VRegIdx) {
        let start = vreg.as_usize() * self.vlen.bytes();
        let end = start + self.vlen.bytes();
        self.data[start..end].fill(0);
    }
}

impl std::fmt::Debug for Vpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vpr").field("vlen", &self.vlen).field("data_len", &self.data.len()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::units::vpu::types::Vlmul;

    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    #[test]
    fn test_new_zeroed() {
        let vpr = vpr128();
        assert_eq!(vpr.data.len(), 32 * 16); // 32 regs * 16 bytes
        assert!(vpr.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_element_read_write_e8() {
        let mut vpr = vpr128();
        let v1 = VRegIdx::new(1);
        vpr.write_element(v1, ElemIdx::new(0), Sew::E8, 0xAB);
        assert_eq!(vpr.read_element(v1, ElemIdx::new(0), Sew::E8), 0xAB);
        // Other elements should be zero
        assert_eq!(vpr.read_element(v1, ElemIdx::new(1), Sew::E8), 0);
    }

    #[test]
    fn test_element_read_write_e32() {
        let mut vpr = vpr128();
        let v5 = VRegIdx::new(5);
        vpr.write_element(v5, ElemIdx::new(2), Sew::E32, 0xDEAD_BEEF);
        assert_eq!(vpr.read_element(v5, ElemIdx::new(2), Sew::E32), 0xDEAD_BEEF);
    }

    #[test]
    fn test_element_read_write_e64() {
        let mut vpr = vpr128();
        let v0 = VRegIdx::new(0);
        vpr.write_element(v0, ElemIdx::new(1), Sew::E64, 0x1234_5678_9ABC_DEF0);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(1), Sew::E64), 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn test_element_truncation() {
        let mut vpr = vpr128();
        let v0 = VRegIdx::new(0);
        // Write a value larger than SEW=8 — only low 8 bits stored
        vpr.write_element(v0, ElemIdx::new(0), Sew::E8, 0x1FF);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(0), Sew::E8), 0xFF);
    }

    #[test]
    fn test_mask_bit_read_write() {
        let mut vpr = vpr128();
        let v0 = VRegIdx::new(0);

        // Set bit 5
        vpr.write_mask_bit(v0, ElemIdx::new(5), true);
        assert!(vpr.read_mask_bit(v0, ElemIdx::new(5)));
        assert!(!vpr.read_mask_bit(v0, ElemIdx::new(4)));
        assert!(!vpr.read_mask_bit(v0, ElemIdx::new(6)));

        // Clear bit 5
        vpr.write_mask_bit(v0, ElemIdx::new(5), false);
        assert!(!vpr.read_mask_bit(v0, ElemIdx::new(5)));
    }

    #[test]
    fn test_read_write_bytes() {
        let mut vpr = vpr128();
        let v3 = VRegIdx::new(3);
        let data: Vec<u8> = (0..16).collect();
        vpr.write_bytes(v3, &data);
        assert_eq!(vpr.read_bytes(v3), &data[..]);
    }

    #[test]
    fn test_register_group() {
        let mut vpr = vpr128();
        let group = Vlmul::M4.group_regs();
        let v0 = VRegIdx::new(0);

        let data: Vec<u8> = (0..64).collect(); // 4 regs * 16 bytes
        vpr.write_group(v0, group, &data);
        assert_eq!(vpr.read_group(v0, group), &data[..]);

        // Individual register reads should match
        assert_eq!(vpr.read_bytes(VRegIdx::new(0)), &data[0..16]);
        assert_eq!(vpr.read_bytes(VRegIdx::new(1)), &data[16..32]);
        assert_eq!(vpr.read_bytes(VRegIdx::new(2)), &data[32..48]);
        assert_eq!(vpr.read_bytes(VRegIdx::new(3)), &data[48..64]);
    }

    #[test]
    #[should_panic(expected = "not aligned")]
    fn test_register_group_unaligned_panics() {
        let vpr = vpr128();
        let group = Vlmul::M4.group_regs();
        let v1 = VRegIdx::new(1); // not aligned to 4
        let _ = vpr.read_group(v1, group);
    }

    #[test]
    fn test_copy_reg() {
        let mut vpr = vpr128();
        let v2 = VRegIdx::new(2);
        let v7 = VRegIdx::new(7);
        let data: Vec<u8> = (100..116).collect();
        vpr.write_bytes(v2, &data);
        vpr.copy_reg(v7, v2);
        assert_eq!(vpr.read_bytes(v7), &data[..]);
    }

    #[test]
    fn test_zero_reg() {
        let mut vpr = vpr128();
        let v1 = VRegIdx::new(1);
        vpr.write_element(v1, ElemIdx::new(0), Sew::E64, u64::MAX);
        vpr.zero_reg(v1);
        assert_eq!(vpr.read_element(v1, ElemIdx::new(0), Sew::E64), 0);
    }

    #[test]
    fn test_different_sew_same_data() {
        let mut vpr = vpr128();
        let v0 = VRegIdx::new(0);
        // Write 0x04030201 at E32 element 0
        vpr.write_element(v0, ElemIdx::new(0), Sew::E32, 0x04030201);
        // Read back as E8 elements
        assert_eq!(vpr.read_element(v0, ElemIdx::new(0), Sew::E8), 0x01);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(1), Sew::E8), 0x02);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(2), Sew::E8), 0x03);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(3), Sew::E8), 0x04);
        // Read as E16
        assert_eq!(vpr.read_element(v0, ElemIdx::new(0), Sew::E16), 0x0201);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(1), Sew::E16), 0x0403);
    }
}

//! Architectural Vector Register File (VPR).
//!
//! Provides 32 × VLEN-bit vector registers with element-wise access at all SEW
//! values and register group operations for LMUL > 1.

use super::vec_storage::VecRegStorage;
use crate::isa::rvv::{ElemIdx, LmulGroup, Sew, VRegIdx, Vlen};

/// Architectural vector register file: 32 registers, each VLEN bits wide.
#[derive(Clone, Debug)]
pub struct Vpr {
    storage: VecRegStorage,
}

impl Vpr {
    /// Create a new VPR with all registers zeroed.
    pub fn new(vlen: Vlen) -> Self {
        Self { storage: VecRegStorage::new(VRegIdx::COUNT, vlen) }
    }

    /// All 32 registers' bytes, register 0 first.
    pub fn bytes(&self) -> &[u8] {
        self.storage.bytes()
    }

    /// Overwrites all 32 registers from `bytes`, laid out as [`Self::bytes`].
    pub fn set_bytes(&mut self, bytes: &[u8]) {
        let data = self.storage.bytes_mut();
        let len = data.len().min(bytes.len());
        data[..len].copy_from_slice(&bytes[..len]);
    }

    /// Returns the configured VLEN.
    #[inline]
    pub const fn vlen(&self) -> Vlen {
        self.storage.vlen()
    }

    /// Read a single element from a vector register, zero-extended to u64.
    pub fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        self.storage.read_element(vreg.as_usize(), index, sew)
    }

    /// Write a single element to a vector register, truncating `val` to SEW bits.
    pub fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        self.storage.write_element(vreg.as_usize(), index, sew, val);
    }

    /// Read the raw bytes of a single vector register.
    pub fn read_bytes(&self, vreg: VRegIdx) -> &[u8] {
        self.storage.registers(vreg.as_usize(), 1)
    }

    /// Write raw bytes to a single vector register.
    ///
    /// # Panics
    ///
    /// Panics if `data.len() != vlen.bytes()`.
    pub fn write_bytes(&mut self, vreg: VRegIdx, data: &[u8]) {
        self.storage.registers_mut(vreg.as_usize(), 1).copy_from_slice(data);
    }

    /// Read a single mask bit from a vector register.
    ///
    /// Mask bit `i` is stored as bit `i % 8` of byte `i / 8` within the register.
    pub fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool {
        self.storage.read_mask_bit(vreg.as_usize(), index)
    }

    /// Write a single mask bit in a vector register.
    pub fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool) {
        self.storage.write_mask_bit(vreg.as_usize(), index, val);
    }

    /// Read the raw bytes of a register group (LMUL consecutive registers).
    ///
    /// # Panics
    ///
    /// Panics if `base` is not aligned to the group size.
    pub fn read_group(&self, base: VRegIdx, group: LmulGroup) -> &[u8] {
        assert_group_aligned(base, group);
        self.storage.registers(base.as_usize(), group.regs_usize())
    }

    /// Write raw bytes to a register group (LMUL consecutive registers).
    ///
    /// # Panics
    ///
    /// Panics if `base` is not aligned or `data.len()` doesn't match.
    pub fn write_group(&mut self, base: VRegIdx, group: LmulGroup, data: &[u8]) {
        assert_group_aligned(base, group);
        self.storage.registers_mut(base.as_usize(), group.regs_usize()).copy_from_slice(data);
    }

    /// Copy one register to another.
    pub fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx) {
        self.storage.copy_register(dst.as_usize(), src.as_usize());
    }

    /// Zero a single register.
    pub fn zero_reg(&mut self, vreg: VRegIdx) {
        self.storage.registers_mut(vreg.as_usize(), 1).fill(0);
    }
}

fn assert_group_aligned(base: VRegIdx, group: LmulGroup) {
    assert!(
        base.is_aligned(group),
        "register group base {} not aligned to LMUL group size {}",
        base.as_u8(),
        group.regs()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::rvv::Vlmul;

    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    #[test]
    fn test_new_zeroed() {
        let vpr = vpr128();
        assert_eq!(vpr.bytes().len(), 32 * 16);
        assert!(vpr.bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_element_read_write_e8() {
        let mut vpr = vpr128();
        let v1 = VRegIdx::new(1);
        vpr.write_element(v1, ElemIdx::new(0), Sew::E8, 0xAB);
        assert_eq!(vpr.read_element(v1, ElemIdx::new(0), Sew::E8), 0xAB);
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
        vpr.write_element(v0, ElemIdx::new(0), Sew::E8, 0x1FF);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(0), Sew::E8), 0xFF);
    }

    #[test]
    fn test_mask_bit_read_write() {
        let mut vpr = vpr128();
        let v0 = VRegIdx::new(0);

        vpr.write_mask_bit(v0, ElemIdx::new(5), true);
        assert!(vpr.read_mask_bit(v0, ElemIdx::new(5)));
        assert!(!vpr.read_mask_bit(v0, ElemIdx::new(4)));
        assert!(!vpr.read_mask_bit(v0, ElemIdx::new(6)));

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

        let data: Vec<u8> = (0..64).collect();
        vpr.write_group(v0, group, &data);
        assert_eq!(vpr.read_group(v0, group), &data[..]);

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
        let v1 = VRegIdx::new(1);
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
        vpr.write_element(v0, ElemIdx::new(0), Sew::E32, 0x04030201);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(0), Sew::E8), 0x01);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(1), Sew::E8), 0x02);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(2), Sew::E8), 0x03);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(3), Sew::E8), 0x04);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(0), Sew::E16), 0x0201);
        assert_eq!(vpr.read_element(v0, ElemIdx::new(1), Sew::E16), 0x0403);
    }
}

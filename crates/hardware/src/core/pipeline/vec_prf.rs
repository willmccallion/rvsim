//! Vector Physical Register File for out-of-order execution.
//!
//! Each vector physical register stores VLEN bits of data plus a ready bit.
//! VecPhysReg(0) is not hardwired (vectors have no zero register), but slot 0
//! is used as a sentinel for "no allocation" and is always ready.
//!
//! `VecPrfView` implements [`VectorRegFile`] by translating architectural
//! VRegIdx to physical VecPhysReg via a mapping table, enabling the VPU
//! execution functions to operate transparently on physical registers.

use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::vpu::types::{ElemIdx, Sew, VRegIdx, VecPhysReg, Vlen};

/// Vector physical register file: flat byte storage with per-register ready bits.
#[derive(Debug)]
pub struct VecPhysRegFile {
    /// Contiguous storage: `prf_vpr_size * vlen_bytes` bytes.
    data: Vec<u8>,
    /// Per-register ready bits.
    ready: Vec<bool>,
    /// VLEN configuration.
    vlen: Vlen,
    /// Total number of physical registers.
    total: usize,
}

impl VecPhysRegFile {
    /// Create a new vector PRF.
    ///
    /// `total` = total physical registers (including architectural slots 0..32).
    /// VecPhysReg(0) is always ready (sentinel for "no allocation").
    pub fn new(total: usize, vlen: Vlen) -> Self {
        let mut ready = vec![false; total];
        if total > 0 {
            ready[0] = true; // Slot 0 sentinel always ready
        }
        Self { data: vec![0u8; total * vlen.bytes()], ready, vlen, total }
    }

    /// Read a single element from a physical register.
    pub fn read_element(&self, p: VecPhysReg, index: ElemIdx, sew: Sew) -> u64 {
        let off = self.offset(p, index, sew);
        let bytes = sew.bytes();
        debug_assert!(off + bytes <= self.data.len(), "VecPRF read out of bounds");
        let mut val = 0u64;
        for i in 0..bytes {
            val |= (self.data[off + i] as u64) << (i * 8);
        }
        val
    }

    /// Write a single element to a physical register.
    pub fn write_element(&mut self, p: VecPhysReg, index: ElemIdx, sew: Sew, val: u64) {
        let off = self.offset(p, index, sew);
        let bytes = sew.bytes();
        debug_assert!(off + bytes <= self.data.len(), "VecPRF write out of bounds");
        for i in 0..bytes {
            self.data[off + i] = (val >> (i * 8)) as u8;
        }
    }

    /// Read a single mask bit from a physical register.
    pub fn read_mask_bit(&self, p: VecPhysReg, index: ElemIdx) -> bool {
        let base = p.as_usize() * self.vlen.bytes();
        let byte_off = base + index.as_usize() / 8;
        let bit_off = index.as_usize() % 8;
        debug_assert!(byte_off < self.data.len(), "VecPRF mask read out of bounds");
        (self.data[byte_off] >> bit_off) & 1 != 0
    }

    /// Write a single mask bit in a physical register.
    pub fn write_mask_bit(&mut self, p: VecPhysReg, index: ElemIdx, val: bool) {
        let base = p.as_usize() * self.vlen.bytes();
        let byte_off = base + index.as_usize() / 8;
        let bit_off = index.as_usize() % 8;
        debug_assert!(byte_off < self.data.len(), "VecPRF mask write out of bounds");
        if val {
            self.data[byte_off] |= 1 << bit_off;
        } else {
            self.data[byte_off] &= !(1 << bit_off);
        }
    }

    /// Read the raw bytes of a physical register.
    pub fn read_bytes(&self, p: VecPhysReg) -> &[u8] {
        let start = p.as_usize() * self.vlen.bytes();
        &self.data[start..start + self.vlen.bytes()]
    }

    /// Write raw bytes to a physical register.
    ///
    /// # Panics
    ///
    /// Panics if `data.len() != vlen.bytes()`.
    pub fn write_bytes(&mut self, p: VecPhysReg, data: &[u8]) {
        assert_eq!(data.len(), self.vlen.bytes(), "VecPRF write_bytes: length mismatch");
        let start = p.as_usize() * self.vlen.bytes();
        self.data[start..start + self.vlen.bytes()].copy_from_slice(data);
    }

    /// Copy one physical register to another.
    pub fn copy_reg(&mut self, dst: VecPhysReg, src: VecPhysReg) {
        if dst == src {
            return;
        }
        let vlen_bytes = self.vlen.bytes();
        let src_start = src.as_usize() * vlen_bytes;
        let dst_start = dst.as_usize() * vlen_bytes;
        self.data.copy_within(src_start..src_start + vlen_bytes, dst_start);
    }

    /// Returns true if the physical register is ready (result available).
    #[inline]
    pub fn is_ready(&self, p: VecPhysReg) -> bool {
        let idx = p.as_usize();
        if idx < self.ready.len() { self.ready[idx] } else { false }
    }

    /// Mark a physical register as not-ready (allocated but result pending).
    #[inline]
    pub fn allocate(&mut self, p: VecPhysReg) {
        let idx = p.as_usize();
        if idx < self.ready.len() {
            self.ready[idx] = false;
        }
    }

    /// Mark a physical register as ready (result available).
    #[inline]
    pub fn mark_ready(&mut self, p: VecPhysReg) {
        let idx = p.as_usize();
        if idx < self.ready.len() {
            self.ready[idx] = true;
        }
    }

    /// Mark the first `num_arch` identity-mapped slots as ready.
    pub fn mark_arch_ready(&mut self, num_arch: usize) {
        let limit = num_arch.min(self.ready.len());
        for i in 0..limit {
            self.ready[i] = true;
        }
    }

    /// Returns the VLEN configuration.
    #[inline]
    pub const fn vlen(&self) -> Vlen {
        self.vlen
    }

    /// Total number of physical vector registers.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.total
    }

    /// Byte offset for a given physical register and element.
    #[inline]
    const fn offset(&self, p: VecPhysReg, index: ElemIdx, sew: Sew) -> usize {
        p.as_usize() * self.vlen.bytes() + index.as_usize() * sew.bytes()
    }
}

/// View into the vector PRF with architectural-to-physical register mapping.
///
/// Implements [`VectorRegFile`], allowing VPU execute functions to transparently
/// access physical registers via architectural `VRegIdx` names.
#[derive(Debug)]
pub struct VecPrfView<'a> {
    /// The underlying physical register file.
    prf: &'a mut VecPhysRegFile,
    /// Mapping from architectural `VRegIdx` (0..31) to physical `VecPhysReg`.
    mapping: [VecPhysReg; 32],
}

impl<'a> VecPrfView<'a> {
    /// Create a new view with the given mapping.
    pub const fn new(prf: &'a mut VecPhysRegFile, mapping: [VecPhysReg; 32]) -> Self {
        Self { prf, mapping }
    }

    /// Translate an architectural `VRegIdx` to a physical `VecPhysReg`.
    #[inline]
    const fn translate(&self, vreg: VRegIdx) -> VecPhysReg {
        self.mapping[vreg.as_usize()]
    }
}

impl VectorRegFile for VecPrfView<'_> {
    #[inline]
    fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        self.prf.read_element(self.translate(vreg), index, sew)
    }

    #[inline]
    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        self.prf.write_element(self.translate(vreg), index, sew, val);
    }

    #[inline]
    fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool {
        self.prf.read_mask_bit(self.translate(vreg), index)
    }

    #[inline]
    fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool) {
        self.prf.write_mask_bit(self.translate(vreg), index, val);
    }

    #[inline]
    fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx) {
        self.prf.copy_reg(self.translate(dst), self.translate(src));
    }

    #[inline]
    fn vlen(&self) -> Vlen {
        self.prf.vlen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_prf() -> VecPhysRegFile {
        VecPhysRegFile::new(64, Vlen::new_unchecked(128))
    }

    #[test]
    fn test_element_lifecycle() {
        let mut prf = make_prf();
        let p5 = VecPhysReg::new(5);

        prf.write_element(p5, ElemIdx::new(0), Sew::E32, 0xCAFEBABE);
        assert_eq!(prf.read_element(p5, ElemIdx::new(0), Sew::E32), 0xCAFEBABE);
        assert_eq!(prf.read_element(p5, ElemIdx::new(1), Sew::E32), 0);
    }

    #[test]
    fn test_mask_bits() {
        let mut prf = make_prf();
        let p3 = VecPhysReg::new(3);

        prf.write_mask_bit(p3, ElemIdx::new(7), true);
        assert!(prf.read_mask_bit(p3, ElemIdx::new(7)));
        assert!(!prf.read_mask_bit(p3, ElemIdx::new(6)));

        prf.write_mask_bit(p3, ElemIdx::new(7), false);
        assert!(!prf.read_mask_bit(p3, ElemIdx::new(7)));
    }

    #[test]
    fn test_ready_bits() {
        let mut prf = make_prf();
        let p10 = VecPhysReg::new(10);

        assert!(!prf.is_ready(p10));
        prf.mark_ready(p10);
        assert!(prf.is_ready(p10));
        prf.allocate(p10);
        assert!(!prf.is_ready(p10));
    }

    #[test]
    fn test_mark_arch_ready() {
        let mut prf = make_prf();
        prf.mark_arch_ready(32);
        for i in 0..32 {
            assert!(prf.is_ready(VecPhysReg::new(i)));
        }
        assert!(!prf.is_ready(VecPhysReg::new(32)));
    }

    #[test]
    fn test_read_write_bytes() {
        let mut prf = make_prf();
        let p1 = VecPhysReg::new(1);
        let data: Vec<u8> = (0..16).collect();
        prf.write_bytes(p1, &data);
        assert_eq!(prf.read_bytes(p1), &data[..]);
    }

    #[test]
    fn test_copy_reg() {
        let mut prf = make_prf();
        let p1 = VecPhysReg::new(1);
        let p2 = VecPhysReg::new(2);
        prf.write_element(p1, ElemIdx::new(0), Sew::E64, 0x1234_5678_9ABC_DEF0);
        prf.copy_reg(p2, p1);
        assert_eq!(prf.read_element(p2, ElemIdx::new(0), Sew::E64), 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn test_vec_prf_view() {
        let mut prf = make_prf();
        let p40 = VecPhysReg::new(40);
        let p41 = VecPhysReg::new(41);

        // Set up mapping: v1 → p40, v2 → p41
        let mut mapping = [VecPhysReg::ZERO; 32];
        for (i, slot) in mapping.iter_mut().enumerate() {
            *slot = VecPhysReg::new(i as u16); // identity
        }
        mapping[1] = p40;
        mapping[2] = p41;

        {
            let mut view = VecPrfView::new(&mut prf, mapping);

            // Write through trait view
            VectorRegFile::write_element(&mut view, VRegIdx::new(1), ElemIdx::new(0), Sew::E32, 42);
            assert_eq!(
                VectorRegFile::read_element(&view, VRegIdx::new(1), ElemIdx::new(0), Sew::E32),
                42
            );

            // copy_reg through view
            VectorRegFile::copy_reg(&mut view, VRegIdx::new(2), VRegIdx::new(1));
            assert_eq!(
                VectorRegFile::read_element(&view, VRegIdx::new(2), ElemIdx::new(0), Sew::E32),
                42
            );
        }

        // Verify data went to physical registers
        assert_eq!(prf.read_element(p40, ElemIdx::new(0), Sew::E32), 42);
        assert_eq!(prf.read_element(p41, ElemIdx::new(0), Sew::E32), 42);
    }

    #[test]
    fn test_vec_prf_view_same_results_as_vpr() {
        use crate::core::arch::vpr::Vpr;

        let vlen = Vlen::new_unchecked(128);
        let mut vpr = Vpr::new(vlen);
        let mut prf = VecPhysRegFile::new(64, vlen);

        // Identity mapping for VecPrfView
        let mut mapping = [VecPhysReg::ZERO; 32];
        for (i, slot) in mapping.iter_mut().enumerate() {
            *slot = VecPhysReg::new(i as u16);
        }

        // Write same data to both
        let v3 = VRegIdx::new(3);
        VectorRegFile::write_element(&mut vpr, v3, ElemIdx::new(0), Sew::E32, 0xDEAD);
        VectorRegFile::write_element(&mut vpr, v3, ElemIdx::new(1), Sew::E32, 0xBEEF);

        {
            let mut view = VecPrfView::new(&mut prf, mapping);
            VectorRegFile::write_element(&mut view, v3, ElemIdx::new(0), Sew::E32, 0xDEAD);
            VectorRegFile::write_element(&mut view, v3, ElemIdx::new(1), Sew::E32, 0xBEEF);
        }

        // Read back and compare
        let view = VecPrfView::new(&mut prf, mapping);
        for i in 0..4 {
            let idx = ElemIdx::new(i);
            assert_eq!(
                VectorRegFile::read_element(&vpr, v3, idx, Sew::E32),
                VectorRegFile::read_element(&view, v3, idx, Sew::E32),
                "mismatch at element {i}"
            );
        }
    }
}

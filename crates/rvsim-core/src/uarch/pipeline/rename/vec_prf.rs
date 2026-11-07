//! Vector Physical Register File for out-of-order execution.
//!
//! Each vector physical register stores VLEN bits of data plus a ready bit.
//! Vectors have no hardwired register: slot 0 holds v0 from reset and is
//! renamed and reclaimed like any other.
//!
//! `VecPrfView` implements [`VectorRegFile`] by translating architectural
//! `VRegIdx` to physical `VecPhysReg` via a mapping table, enabling the VPU
//! execution functions to operate transparently on physical registers.

use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::rvv::{ElemIdx, MAX_GROUP_REGS, Sew, VRegIdx, Vlen};
use crate::uarch::pipeline::rename::free_list::PhysRegister;

/// Vector physical register index (O3 backend).
/// NOT interchangeable with `PhysReg` (scalar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VecPhysReg(u16);

impl VecPhysReg {
    /// Physical register 0, which also fills the unused slots of a
    /// register group.
    pub const ZERO: Self = Self(0);

    /// Create a new vector physical register index.
    #[inline(always)]
    pub const fn new(val: u16) -> Self {
        Self(val)
    }

    /// Returns the index as `u16`.
    #[inline(always)]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns the index as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// A register group's physical registers: the first `len` of up to
/// [`MAX_GROUP_REGS`], the rest `VecPhysReg::ZERO`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VecRegGroup {
    regs: [VecPhysReg; MAX_GROUP_REGS],
    len: u8,
}

impl VecRegGroup {
    /// A group of no registers.
    pub const EMPTY: Self = Self { regs: [VecPhysReg::ZERO; MAX_GROUP_REGS], len: 0 };

    /// The group of `regs`, at most [`MAX_GROUP_REGS`] of them.
    #[must_use]
    pub fn from_slice(regs: &[VecPhysReg]) -> Self {
        let mut group = Self::EMPTY;
        group.regs[..regs.len()].copy_from_slice(regs);
        group.len = regs.len() as u8;
        group
    }

    /// The registers in the group, in order.
    #[must_use]
    pub fn as_slice(&self) -> &[VecPhysReg] {
        &self.regs[..usize::from(self.len)]
    }

    /// How many registers the group has.
    #[must_use]
    pub const fn len(&self) -> u8 {
        self.len
    }

    /// Whether the group has no registers.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The register at `slot`, `VecPhysReg::ZERO` past the group's end.
    #[must_use]
    pub const fn get(&self, slot: usize) -> VecPhysReg {
        self.regs[slot]
    }
}

impl PhysRegister for VecPhysReg {
    #[inline]
    fn is_hardwired(self) -> bool {
        false
    }

    #[inline]
    fn from_index(idx: u16) -> Self {
        Self(idx)
    }
}

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
    pub fn new(total: usize, vlen: Vlen) -> Self {
        Self { data: vec![0u8; total * vlen.bytes()], ready: vec![false; total], vlen, total }
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
        // LMUL > 1: route element idx to the correct register in the group.
        let elems_per_phys = (self.prf.vlen().bytes() / sew.bytes()).max(1);
        let phys_offset = index.as_usize() / elems_per_phys;
        let local_idx = ElemIdx::new(index.as_usize() % elems_per_phys);
        let actual_vreg = VRegIdx::new(vreg.as_u8() + phys_offset as u8);
        self.prf.read_element(self.translate(actual_vreg), local_idx, sew)
    }

    #[inline]
    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        let elems_per_phys = (self.prf.vlen().bytes() / sew.bytes()).max(1);
        let phys_offset = index.as_usize() / elems_per_phys;
        let local_idx = ElemIdx::new(index.as_usize() % elems_per_phys);
        let actual_vreg = VRegIdx::new(vreg.as_u8() + phys_offset as u8);
        self.prf.write_element(self.translate(actual_vreg), local_idx, sew, val);
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
        use crate::arch::regs::vpr::Vpr;

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

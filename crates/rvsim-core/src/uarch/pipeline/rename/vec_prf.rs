//! Vector Physical Register File for out-of-order execution.
//!
//! Each vector physical register stores VLEN bits of data plus a ready bit.
//! Vectors have no hardwired register: slot 0 holds v0 from reset and is
//! renamed and reclaimed like any other.
//!
//! `VecPrfView` implements [`VectorRegFile`] by translating architectural
//! `VRegIdx` to physical `VecPhysReg` via a mapping table, enabling the VPU
//! execution functions to operate transparently on physical registers.

use crate::arch::regs::vec_storage::VecRegStorage;
use crate::common::ids::impl_id_from_index;
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::rvv::{ElemIdx, MAX_GROUP_REGS, Sew, VRegIdx, Vlen};
use crate::uarch::pipeline::rename::free_list::PhysRegister;

/// Vector physical register index (O3 backend).
/// NOT interchangeable with `PhysReg` (scalar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VecPhysReg(u16);

impl_id_from_index!(VecPhysReg, u16);

impl VecPhysReg {
    /// Physical register 0, which also fills the unused slots of a
    /// register group.
    pub const ZERO: Self = Self(0);

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
    #[expect(clippy::cast_possible_truncation, reason = "the copy bounds it by MAX_GROUP_REGS")]
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
}

/// Vector physical register file: flat byte storage with per-register ready bits.
#[derive(Debug)]
pub struct VecPhysRegFile {
    storage: VecRegStorage,
    /// Per-register ready bits.
    ready: Vec<bool>,
    /// Total number of physical registers.
    total: usize,
}

impl VecPhysRegFile {
    /// Create a new vector PRF.
    ///
    /// `total` = total physical registers (including architectural slots 0..32).
    pub fn new(total: usize, vlen: Vlen) -> Self {
        Self { storage: VecRegStorage::new(total, vlen), ready: vec![false; total], total }
    }

    /// Read a single element from a physical register.
    pub fn read_element(&self, p: VecPhysReg, index: ElemIdx, sew: Sew) -> u64 {
        self.storage.read_element(p.as_usize(), index, sew)
    }

    /// Write a single element to a physical register.
    pub fn write_element(&mut self, p: VecPhysReg, index: ElemIdx, sew: Sew, val: u64) {
        self.storage.write_element(p.as_usize(), index, sew, val);
    }

    /// Read a single mask bit from a physical register.
    pub fn read_mask_bit(&self, p: VecPhysReg, index: ElemIdx) -> bool {
        self.storage.read_mask_bit(p.as_usize(), index)
    }

    /// Write a single mask bit in a physical register.
    pub fn write_mask_bit(&mut self, p: VecPhysReg, index: ElemIdx, val: bool) {
        self.storage.write_mask_bit(p.as_usize(), index, val);
    }

    /// Read the raw bytes of a physical register.
    pub fn read_bytes(&self, p: VecPhysReg) -> &[u8] {
        self.storage.registers(p.as_usize(), 1)
    }

    /// Write raw bytes to a physical register.
    ///
    /// # Panics
    ///
    /// Panics if `data.len() != vlen.bytes()`.
    pub fn write_bytes(&mut self, p: VecPhysReg, data: &[u8]) {
        self.storage.registers_mut(p.as_usize(), 1).copy_from_slice(data);
    }

    /// Copy one physical register to another.
    pub fn copy_reg(&mut self, dst: VecPhysReg, src: VecPhysReg) {
        self.storage.copy_register(dst.as_usize(), src.as_usize());
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
        self.storage.vlen()
    }

    /// Total number of physical vector registers.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.total
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

    /// The physical register holding element `index` of the group at
    /// `vreg`, and the element's index within it.
    fn locate(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> (VecPhysReg, ElemIdx) {
        let elems_per_reg = (self.prf.vlen().bytes() / sew.bytes()).max(1);
        let reg_in_group = index.as_usize() / elems_per_reg;
        let member = vreg.offset(reg_in_group);
        (self.translate(member), ElemIdx::new(index.as_usize() % elems_per_reg))
    }
}

impl VectorRegFile for VecPrfView<'_> {
    #[inline]
    fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        let (reg, local) = self.locate(vreg, index, sew);
        self.prf.read_element(reg, local, sew)
    }

    #[inline]
    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        let (reg, local) = self.locate(vreg, index, sew);
        self.prf.write_element(reg, local, sew, val);
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
        let p5 = VecPhysReg::from_index(5);

        prf.write_element(p5, ElemIdx::new(0), Sew::E32, 0xCAFEBABE);
        assert_eq!(prf.read_element(p5, ElemIdx::new(0), Sew::E32), 0xCAFEBABE);
        assert_eq!(prf.read_element(p5, ElemIdx::new(1), Sew::E32), 0);
    }

    #[test]
    fn test_mask_bits() {
        let mut prf = make_prf();
        let p3 = VecPhysReg::from_index(3);

        prf.write_mask_bit(p3, ElemIdx::new(7), true);
        assert!(prf.read_mask_bit(p3, ElemIdx::new(7)));
        assert!(!prf.read_mask_bit(p3, ElemIdx::new(6)));

        prf.write_mask_bit(p3, ElemIdx::new(7), false);
        assert!(!prf.read_mask_bit(p3, ElemIdx::new(7)));
    }

    #[test]
    fn test_ready_bits() {
        let mut prf = make_prf();
        let p10 = VecPhysReg::from_index(10);

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
            assert!(prf.is_ready(VecPhysReg::from_index(i)));
        }
        assert!(!prf.is_ready(VecPhysReg::from_index(32)));
    }

    #[test]
    fn test_read_write_bytes() {
        let mut prf = make_prf();
        let p1 = VecPhysReg::from_index(1);
        let data: Vec<u8> = (0..16).collect();
        prf.write_bytes(p1, &data);
        assert_eq!(prf.read_bytes(p1), &data[..]);
    }

    #[test]
    fn test_copy_reg() {
        let mut prf = make_prf();
        let p1 = VecPhysReg::from_index(1);
        let p2 = VecPhysReg::from_index(2);
        prf.write_element(p1, ElemIdx::new(0), Sew::E64, 0x1234_5678_9ABC_DEF0);
        prf.copy_reg(p2, p1);
        assert_eq!(prf.read_element(p2, ElemIdx::new(0), Sew::E64), 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn test_vec_prf_view() {
        let mut prf = make_prf();
        let p40 = VecPhysReg::from_index(40);
        let p41 = VecPhysReg::from_index(41);

        // Set up mapping: v1 → p40, v2 → p41
        let mut mapping = [VecPhysReg::ZERO; 32];
        for (i, slot) in mapping.iter_mut().enumerate() {
            *slot = VecPhysReg::from_index(i); // identity
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
            *slot = VecPhysReg::from_index(i);
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

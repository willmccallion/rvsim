//! Vector results held back until commit.
//!
//! A backend without vector renaming executes a vector instruction against
//! a [`ShadowVpr`]: a private copy of the architectural registers that
//! remembers which registers the instruction wrote. The writes come out as
//! [`VectorWrites`], travel on the ROB entry, and land in the architectural
//! file when the instruction retires, so execute never touches
//! architectural state.

use crate::arch::regs::vpr::Vpr;
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlen};

/// One element a vector load returned, addressed within its register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementWrite {
    /// The destination register.
    pub reg: VRegIdx,
    /// The element's index within `reg`.
    pub index: ElemIdx,
    /// The element's width.
    pub eew: Sew,
    /// The element's value.
    pub data: u64,
}

/// What a retiring vector instruction writes to the architectural registers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VectorWrites {
    /// Whole registers an arithmetic, mask, permute or crypto op produced.
    pub registers: Vec<(VRegIdx, Box<[u8]>)>,
    /// Elements a load returned.
    pub elements: Vec<ElementWrite>,
}

impl VectorWrites {
    /// Lands every write in `vpr`.
    pub fn apply(&self, vpr: &mut Vpr) {
        for (reg, bytes) in &self.registers {
            vpr.write_bytes(*reg, bytes);
        }
        for element in &self.elements {
            vpr.write_element(element.reg, element.index, element.eew, element.data);
        }
    }
}

/// A private copy of the architectural vector registers that records which
/// registers an instruction writes.
#[derive(Clone, Debug)]
pub struct ShadowVpr {
    vpr: Vpr,
    written: u32,
}

impl ShadowVpr {
    /// A shadow of `vpr` with nothing written yet.
    #[must_use]
    pub fn new(vpr: &Vpr) -> Self {
        Self { vpr: vpr.clone(), written: 0 }
    }

    /// The registers written, with their final contents.
    #[must_use]
    pub fn into_writes(self) -> VectorWrites {
        let registers = (0..32u8)
            .filter(|reg| self.written & (1 << reg) != 0)
            .map(|reg| {
                let reg = VRegIdx::new(reg);
                (reg, Box::from(self.vpr.read_bytes(reg)))
            })
            .collect();
        VectorWrites { registers, elements: Vec::new() }
    }

    /// The register `index` of an element group starting at `vreg` lands in.
    fn register_of(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> VRegIdx {
        let elems_per_reg = (self.vpr.vlen().bytes() / sew.bytes()).max(1);
        VRegIdx::new(vreg.as_u8() + (index.as_usize() / elems_per_reg) as u8)
    }

    const fn mark(&mut self, reg: VRegIdx) {
        self.written |= 1 << reg.as_u8();
    }
}

impl VectorRegFile for ShadowVpr {
    #[inline]
    fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        self.vpr.read_element(vreg, index, sew)
    }

    #[inline]
    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        self.mark(self.register_of(vreg, index, sew));
        self.vpr.write_element(vreg, index, sew, val);
    }

    #[inline]
    fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool {
        self.vpr.read_mask_bit(vreg, index)
    }

    #[inline]
    fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool) {
        self.mark(vreg);
        self.vpr.write_mask_bit(vreg, index, val);
    }

    #[inline]
    fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx) {
        self.mark(dst);
        self.vpr.copy_reg(dst, src);
    }

    #[inline]
    fn vlen(&self) -> Vlen {
        self.vpr.vlen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vpr() -> Vpr {
        let mut vpr = Vpr::new(Vlen::new_unchecked(128));
        vpr.write_element(VRegIdx::new(1), ElemIdx::new(0), Sew::E64, 0x1111);
        vpr
    }

    #[test]
    fn writes_stay_in_the_shadow_until_applied() {
        let mut arch = vpr();
        let mut shadow = ShadowVpr::new(&arch);

        shadow.write_element(VRegIdx::new(2), ElemIdx::new(1), Sew::E64, 0x2222);
        assert_eq!(shadow.read_element(VRegIdx::new(2), ElemIdx::new(1), Sew::E64), 0x2222);
        assert_eq!(arch.read_element(VRegIdx::new(2), ElemIdx::new(1), Sew::E64), 0);

        let writes = shadow.into_writes();
        assert_eq!(writes.registers.len(), 1);
        assert_eq!(writes.registers[0].0, VRegIdx::new(2));
        writes.apply(&mut arch);
        assert_eq!(arch.read_element(VRegIdx::new(2), ElemIdx::new(1), Sew::E64), 0x2222);
    }

    #[test]
    fn an_element_past_the_first_register_marks_the_register_it_lands_in() {
        let mut shadow = ShadowVpr::new(&vpr());

        shadow.write_element(VRegIdx::new(4), ElemIdx::new(2), Sew::E64, 7);

        let writes = shadow.into_writes();
        assert_eq!(writes.registers.len(), 1);
        assert_eq!(writes.registers[0].0, VRegIdx::new(5));
    }

    #[test]
    fn element_writes_land_in_place() {
        let mut arch = vpr();
        let writes = VectorWrites {
            registers: Vec::new(),
            elements: vec![ElementWrite {
                reg: VRegIdx::new(3),
                index: ElemIdx::new(1),
                eew: Sew::E32,
                data: 0xABCD,
            }],
        };

        writes.apply(&mut arch);

        assert_eq!(arch.read_element(VRegIdx::new(3), ElemIdx::new(1), Sew::E32), 0xABCD);
        assert_eq!(arch.read_element(VRegIdx::new(3), ElemIdx::new(0), Sew::E32), 0);
    }
}

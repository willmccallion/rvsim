//! Which destination bits a vector instruction filled under the tail- or
//! mask-agnostic policy, for the commit log: those bits may legally differ
//! from another implementation's, the rest may not.

use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlen};

/// The bits of each register an instruction filled with ones because the
/// agnostic policy left them free. Bit `b` of register `r` is bit `b % 8`
/// of byte `b / 8` of `r`'s mask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgnosticFills {
    /// Each register with an agnostic fill, and its bit mask.
    pub registers: Vec<(VRegIdx, Box<[u8]>)>,
}

impl AgnosticFills {
    /// The mask of `reg`'s filled bits, if any were filled.
    #[cfg(feature = "commit-log")]
    #[must_use]
    pub fn of(&self, reg: VRegIdx) -> Option<&[u8]> {
        self.registers.iter().find(|(r, _)| *r == reg).map(|(_, mask)| &**mask)
    }

    fn mark(&mut self, reg: VRegIdx, first_bit: usize, bits: usize, vlen: Vlen) {
        let position = self.registers.iter().position(|(r, _)| *r == reg).unwrap_or_else(|| {
            self.registers.push((reg, vec![0; vlen.bytes()].into_boxed_slice()));
            self.registers.len() - 1
        });
        let mask = &mut self.registers[position].1;
        for bit in first_bit..first_bit + bits {
            if let Some(byte) = mask.get_mut(bit / 8) {
                *byte |= 1 << (bit % 8);
            }
        }
    }
}

/// A register file that records the agnostic fills made through it when
/// recording is on, and otherwise only passes every access through.
#[derive(Debug)]
pub struct AgnosticRecorder<'a, R: VectorRegFile> {
    inner: &'a mut R,
    fills: Option<AgnosticFills>,
}

impl<'a, R: VectorRegFile> AgnosticRecorder<'a, R> {
    /// Wraps `inner`, recording its agnostic fills when `record`.
    pub fn new(inner: &'a mut R, record: bool) -> Self {
        Self { inner, fills: record.then(AgnosticFills::default) }
    }

    /// The fills recorded; `None` when recording was off.
    #[cfg(feature = "commit-log")]
    #[must_use]
    pub fn into_fills(self) -> Option<AgnosticFills> {
        self.fills
    }
}

impl<R: VectorRegFile> VectorRegFile for AgnosticRecorder<'_, R> {
    fn read_element(&self, vreg: VRegIdx, index: ElemIdx, sew: Sew) -> u64 {
        self.inner.read_element(vreg, index, sew)
    }

    fn write_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew, val: u64) {
        self.inner.write_element(vreg, index, sew, val);
    }

    fn read_mask_bit(&self, vreg: VRegIdx, index: ElemIdx) -> bool {
        self.inner.read_mask_bit(vreg, index)
    }

    fn write_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx, val: bool) {
        self.inner.write_mask_bit(vreg, index, val);
    }

    fn copy_reg(&mut self, dst: VRegIdx, src: VRegIdx) {
        self.inner.copy_reg(dst, src);
    }

    fn vlen(&self) -> Vlen {
        self.inner.vlen()
    }

    fn fill_agnostic_element(&mut self, vreg: VRegIdx, index: ElemIdx, sew: Sew) {
        self.inner.fill_agnostic_element(vreg, index, sew);
        let vlen = self.inner.vlen();
        if let Some(fills) = self.fills.as_mut() {
            let elements_per_register = (vlen.bytes() / sew.bytes()).max(1);
            let register = vreg.as_usize() + index.as_usize() / elements_per_register;
            let first_bit = (index.as_usize() % elements_per_register) * sew.bits();
            if let Ok(register) = u8::try_from(register) {
                fills.mark(VRegIdx::new(register), first_bit, sew.bits(), vlen);
            }
        }
    }

    fn fill_agnostic_mask_bit(&mut self, vreg: VRegIdx, index: ElemIdx) {
        self.inner.fill_agnostic_mask_bit(vreg, index);
        let vlen = self.inner.vlen();
        if let Some(fills) = self.fills.as_mut() {
            fills.mark(vreg, index.as_usize(), 1, vlen);
        }
    }
}

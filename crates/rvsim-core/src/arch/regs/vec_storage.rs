//! Byte storage for a file of VLEN-bit vector registers, shared by the
//! architectural register file and the renamed physical one.

use crate::isa::rvv::{ElemIdx, Sew, Vlen};

/// Vector registers of VLEN bits each, stored back to back with each
/// register's elements little-endian, and addressed by register number.
#[derive(Clone)]
pub struct VecRegStorage {
    data: Vec<u8>,
    vlen: Vlen,
}

impl VecRegStorage {
    /// `count` zeroed registers.
    pub fn new(count: usize, vlen: Vlen) -> Self {
        Self { data: vec![0; count * vlen.bytes()], vlen }
    }

    /// The registers' width.
    pub const fn vlen(&self) -> Vlen {
        self.vlen
    }

    /// Every register's bytes, register 0 first.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Every register's bytes, to write.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// The bytes of `count` registers from `first`.
    pub fn registers(&self, first: usize, count: usize) -> &[u8] {
        let start = first * self.vlen.bytes();
        &self.data[start..start + count * self.vlen.bytes()]
    }

    /// The bytes of `count` registers from `first`, to write.
    pub fn registers_mut(&mut self, first: usize, count: usize) -> &mut [u8] {
        let start = first * self.vlen.bytes();
        &mut self.data[start..start + count * self.vlen.bytes()]
    }

    /// Element `index` of `reg` at `sew`, zero-extended.
    pub fn read_element(&self, reg: usize, index: ElemIdx, sew: Sew) -> u64 {
        let mut bytes = [0; 8];
        bytes[..sew.bytes()].copy_from_slice(self.element(reg, index, sew));
        u64::from_le_bytes(bytes)
    }

    /// Writes the low `sew` bits of `value` to element `index` of `reg`.
    pub fn write_element(&mut self, reg: usize, index: ElemIdx, sew: Sew, value: u64) {
        self.element_mut(reg, index, sew).copy_from_slice(&value.to_le_bytes()[..sew.bytes()]);
    }

    /// Mask bit `index` of `reg`: bit `index % 8` of byte `index / 8`.
    pub fn read_mask_bit(&self, reg: usize, index: ElemIdx) -> bool {
        let (byte, bit) = self.mask_position(reg, index);
        self.data[byte] & bit != 0
    }

    /// Sets or clears mask bit `index` of `reg`.
    pub fn write_mask_bit(&mut self, reg: usize, index: ElemIdx, is_set: bool) {
        let (byte, bit) = self.mask_position(reg, index);
        if is_set {
            self.data[byte] |= bit;
        } else {
            self.data[byte] &= !bit;
        }
    }

    /// Copies register `src` over register `dst`.
    pub fn copy_register(&mut self, dst: usize, src: usize) {
        let vlen_bytes = self.vlen.bytes();
        let src_start = src * vlen_bytes;
        self.data.copy_within(src_start..src_start + vlen_bytes, dst * vlen_bytes);
    }

    fn element(&self, reg: usize, index: ElemIdx, sew: Sew) -> &[u8] {
        let start = self.element_start(reg, index, sew);
        &self.data[start..start + sew.bytes()]
    }

    fn element_mut(&mut self, reg: usize, index: ElemIdx, sew: Sew) -> &mut [u8] {
        let start = self.element_start(reg, index, sew);
        &mut self.data[start..start + sew.bytes()]
    }

    const fn element_start(&self, reg: usize, index: ElemIdx, sew: Sew) -> usize {
        reg * self.vlen.bytes() + index.as_usize() * sew.bytes()
    }

    /// The byte holding mask bit `index` of `reg`, and the bit within it.
    const fn mask_position(&self, reg: usize, index: ElemIdx) -> (usize, u8) {
        let byte = reg * self.vlen.bytes() + index.as_usize() / 8;
        (byte, 1 << (index.as_usize() % 8))
    }
}

impl std::fmt::Debug for VecRegStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VecRegStorage")
            .field("vlen", &self.vlen)
            .field("data_len", &self.data.len())
            .finish()
    }
}

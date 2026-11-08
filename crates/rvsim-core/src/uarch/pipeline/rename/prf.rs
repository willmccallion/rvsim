//! Physical Register File (PRF) for out-of-order execution.
//!
//! The PRF decouples architectural register names from physical storage.
//! Each physical register tracks both a value and a ready bit.
//! [`PhysReg::ZERO`] is hardwired zero: always ready, value always 0.

use super::free_list::PhysRegister;
use crate::common::ids::impl_id_from_index;

/// A physical register identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct PhysReg(u16);

impl_id_from_index!(PhysReg, u16);

impl PhysReg {
    /// x0's register, hardwired to zero and always ready. An instruction
    /// with no destination writes it, which discards the result.
    pub const ZERO: Self = Self(0);

    /// Its number, as the register file indexes it.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl PhysRegister for PhysReg {
    #[inline]
    fn is_hardwired(self) -> bool {
        self.0 == 0
    }
}

/// Physical register file with per-register ready bits.
#[derive(Debug)]
pub struct PhysRegFile {
    values: Vec<u64>,
    ready: Vec<bool>,
}

impl PhysRegFile {
    /// Create a new PRF with `total` registers.
    /// [`PhysReg::ZERO`] starts ready, holding 0.
    pub fn new(total: usize) -> Self {
        let mut ready = vec![false; total];
        if let Some(zero) = ready.get_mut(PhysReg::ZERO.index()) {
            *zero = true;
        }
        Self { values: vec![0u64; total], ready }
    }

    /// Write a result to a physical register and mark it ready.
    #[inline]
    pub fn write(&mut self, p: PhysReg, val: u64) {
        let idx = p.0 as usize;
        if idx == 0 {
            return; // PhysReg::ZERO hardwired zero
        }
        if idx < self.values.len() {
            self.values[idx] = val;
            self.ready[idx] = true;
        }
    }

    /// Read the value of a physical register.
    #[inline]
    pub fn read(&self, p: PhysReg) -> u64 {
        let idx = p.0 as usize;
        if idx < self.values.len() { self.values[idx] } else { 0 }
    }

    /// Returns true if the physical register value is available.
    #[inline]
    pub fn is_ready(&self, p: PhysReg) -> bool {
        let idx = p.0 as usize;
        if idx < self.ready.len() { self.ready[idx] } else { false }
    }

    /// Mark the first `num_arch` physical registers as ready (initial arch state).
    ///
    /// Called once at engine init: the identity-mapped registers (`0..num_arch`)
    /// represent the architectural register file at startup. They are all readable
    /// with value 0 until a rename allocates a fresh register for them.
    pub fn mark_arch_ready(&mut self, num_arch: usize) {
        let limit = num_arch.min(self.ready.len());
        for i in 0..limit {
            self.ready[i] = true;
        }
    }

    /// Mark a physical register as not-ready (result pending).
    /// Called at rename time when a new instruction is allocated this
    /// register; [`PhysReg::ZERO`] stays ready.
    #[inline]
    pub fn allocate(&mut self, p: PhysReg) {
        if p == PhysReg::ZERO {
            return;
        }
        let idx = p.index();
        if idx < self.ready.len() {
            self.ready[idx] = false;
        }
    }

    /// Total number of physical registers.
    pub const fn capacity(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phys_reg_zero_always_ready() {
        let prf = PhysRegFile::new(64);
        assert!(prf.is_ready(PhysReg::ZERO));
        assert_eq!(prf.read(PhysReg::ZERO), 0);
    }

    #[test]
    fn test_write_read_lifecycle() {
        let mut prf = PhysRegFile::new(64);
        let p = PhysReg::from_index(5);
        // Initially not ready
        assert!(!prf.is_ready(p));
        // Write marks ready
        prf.write(p, 42);
        assert!(prf.is_ready(p));
        assert_eq!(prf.read(p), 42);
    }

    #[test]
    fn test_allocate_marks_not_ready() {
        let mut prf = PhysRegFile::new(64);
        let p = PhysReg::from_index(10);
        prf.write(p, 99);
        assert!(prf.is_ready(p));
        prf.allocate(p);
        assert!(!prf.is_ready(p));
    }

    #[test]
    fn test_allocate_zero_noop() {
        let mut prf = PhysRegFile::new(64);
        prf.allocate(PhysReg::ZERO);
        assert!(prf.is_ready(PhysReg::ZERO));
    }

    #[test]
    fn test_write_zero_noop() {
        let mut prf = PhysRegFile::new(64);
        prf.write(PhysReg::ZERO, 999);
        assert_eq!(prf.read(PhysReg::ZERO), 0);
    }
}

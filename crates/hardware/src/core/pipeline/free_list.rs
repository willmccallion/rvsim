//! Physical register free list for O3 rename.
//!
//! Tracks which physical registers are available for allocation.
//! On init: arch regs 0..num_arch are "in use" (held in rename map).
//! Regs num_arch..prf_size are free.
//!
//! Generic over `PhysRegister` to support both scalar `PhysReg` and `VecPhysReg`.

use std::collections::VecDeque;

/// Trait for physical register types usable with the free list.
pub trait PhysRegister: Copy + Eq + std::fmt::Debug {
    /// Returns true if this is the zero/sentinel register (never reclaimed).
    fn is_zero(self) -> bool;
    /// Construct a physical register from a raw index.
    fn from_index(idx: u16) -> Self;
}

/// FIFO free list of available physical registers.
#[derive(Debug)]
pub struct FreeList<R: PhysRegister> {
    queue: VecDeque<R>,
    capacity: usize,
}

impl<R: PhysRegister> FreeList<R> {
    /// Create a new free list.
    /// `prf_size` = total physical registers.
    /// `num_arch` = architectural registers (`0..num_arch` are in use at init).
    pub fn new(prf_size: usize, num_arch: usize) -> Self {
        let mut queue = VecDeque::with_capacity(prf_size);
        // Registers num_arch..prf_size are free initially
        for i in num_arch..prf_size {
            queue.push_back(R::from_index(i as u16));
        }
        Self { queue, capacity: prf_size }
    }

    /// Allocate a free physical register. Returns None if no registers are free.
    pub fn allocate(&mut self) -> Option<R> {
        self.queue.pop_front()
    }

    /// Return a physical register to the free list.
    pub fn reclaim(&mut self, p: R) {
        if !p.is_zero() {
            self.queue.push_back(p);
        }
    }

    /// Number of free registers available.
    pub fn available(&self) -> usize {
        self.queue.len()
    }

    /// Total capacity of the physical register file.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::core::pipeline::prf::PhysReg;
    use crate::core::units::vpu::types::VecPhysReg;

    #[test]
    fn test_allocate_reclaim_roundtrip() {
        let mut fl = FreeList::<PhysReg>::new(64, 32);
        assert_eq!(fl.available(), 32); // 64-32 = 32 free

        let p = fl.allocate().unwrap();
        assert_eq!(fl.available(), 31);

        fl.reclaim(p);
        assert_eq!(fl.available(), 32);
    }

    #[test]
    fn test_underflow_returns_none() {
        let mut fl = FreeList::<PhysReg>::new(4, 4); // all in use
        assert_eq!(fl.available(), 0);
        assert!(fl.allocate().is_none());
    }

    #[test]
    fn test_reclaim_zero_noop() {
        let mut fl = FreeList::<PhysReg>::new(8, 4);
        let initial = fl.available();
        fl.reclaim(PhysReg(0)); // x0 should not be reclaimed
        assert_eq!(fl.available(), initial);
    }

    #[test]
    fn test_capacity() {
        let fl = FreeList::<PhysReg>::new(128, 32);
        assert_eq!(fl.capacity(), 128);
        assert_eq!(fl.available(), 96);
    }

    #[test]
    fn test_vec_phys_reg_free_list() {
        let mut fl = FreeList::<VecPhysReg>::new(64, 32);
        assert_eq!(fl.available(), 32);

        let p = fl.allocate().unwrap();
        assert_eq!(p, VecPhysReg::new(32));
        assert_eq!(fl.available(), 31);

        fl.reclaim(p);
        assert_eq!(fl.available(), 32);
    }

    #[test]
    fn test_vec_phys_reg_reclaim_zero_noop() {
        let mut fl = FreeList::<VecPhysReg>::new(8, 4);
        let initial = fl.available();
        fl.reclaim(VecPhysReg::ZERO);
        assert_eq!(fl.available(), initial);
    }
}

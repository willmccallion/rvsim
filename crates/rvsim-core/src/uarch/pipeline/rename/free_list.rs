//! Physical register free list for O3 rename.
//!
//! Tracks which physical registers are available for allocation. The
//! architectural registers' initial mappings are never on it.
//!
//! Generic over `PhysRegister` to support both scalar `PhysReg` and `VecPhysReg`.

use std::collections::VecDeque;

/// Trait for physical register types usable with the free list.
pub trait PhysRegister: Copy + Eq + std::fmt::Debug {
    /// True for a hardwired register, which is never renamed or reclaimed.
    fn is_hardwired(self) -> bool;
}

/// FIFO free list of available physical registers.
#[derive(Debug)]
pub struct FreeList<R: PhysRegister> {
    queue: VecDeque<R>,
}

impl<R: PhysRegister> FreeList<R> {
    /// A free list holding `free`, to be allocated in that order.
    pub fn new(free: impl IntoIterator<Item = R>) -> Self {
        Self { queue: free.into_iter().collect() }
    }

    /// Allocate a free physical register. Returns None if no registers are free.
    pub fn allocate(&mut self) -> Option<R> {
        self.queue.pop_front()
    }

    /// Return a physical register to the free list.
    pub fn reclaim(&mut self, p: R) {
        if !p.is_hardwired() {
            self.queue.push_back(p);
        }
    }

    /// Number of free registers available.
    pub fn available(&self) -> usize {
        self.queue.len()
    }

    /// Reserves `count` registers without allocating them yet, or `None`
    /// when fewer are free. The reservation borrows the list, so nothing
    /// else can take registers before [`Reserved::take`] does.
    pub fn reserve(&mut self, count: usize) -> Option<Reserved<'_, R>> {
        (count <= self.queue.len()).then_some(Reserved { list: self, count })
    }
}

/// `count` registers a [`FreeList::reserve`] call set aside, to take in
/// allocation order.
#[derive(Debug)]
pub struct Reserved<'a, R: PhysRegister> {
    list: &'a mut FreeList<R>,
    count: usize,
}

impl<R: PhysRegister> Reserved<'_, R> {
    /// Allocates the reserved registers, in the order the list hands them
    /// out.
    pub fn take(self) -> impl Iterator<Item = R> {
        self.list.queue.drain(..self.count)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::uarch::pipeline::rename::prf::PhysReg;
    use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;

    #[test]
    fn reclaiming_an_allocated_register_restores_the_count() {
        let mut fl = FreeList::new((32..64).map(PhysReg::from_index));
        assert_eq!(fl.available(), 32); // 64-32 = 32 free

        let p = fl.allocate().unwrap();
        assert_eq!(fl.available(), 31);

        fl.reclaim(p);
        assert_eq!(fl.available(), 32);
    }

    #[test]
    fn allocating_from_an_empty_list_gives_none() {
        let mut fl = FreeList::new((4..4).map(PhysReg::from_index)); // all in use
        assert_eq!(fl.available(), 0);
        assert!(fl.allocate().is_none());
    }

    #[test]
    fn reclaiming_the_scalar_zero_register_is_a_no_op() {
        let mut fl = FreeList::new((4..8).map(PhysReg::from_index));
        let initial = fl.available();
        fl.reclaim(PhysReg::ZERO); // x0 should not be reclaimed
        assert_eq!(fl.available(), initial);
    }

    #[test]
    fn architectural_registers_start_allocated() {
        let fl = FreeList::new((32..128).map(PhysReg::from_index));
        assert_eq!(fl.available(), 96);
    }

    #[test]
    fn a_reservation_hands_out_the_registers_allocate_would() {
        let mut fl = FreeList::new((4..8).map(PhysReg::from_index));
        let expected: Vec<PhysReg> = {
            let mut probe = FreeList::new((4..8).map(PhysReg::from_index));
            (0..3).map(|_| probe.allocate().unwrap()).collect()
        };

        let taken: Vec<PhysReg> = fl.reserve(3).expect("three are free").take().collect();

        assert_eq!(taken, expected);
        assert_eq!(fl.available(), 1);
    }

    #[test]
    fn a_reservation_beyond_the_free_registers_is_refused_and_takes_nothing() {
        let mut fl = FreeList::new((4..8).map(PhysReg::from_index));

        assert!(fl.reserve(5).is_none());
        assert_eq!(fl.available(), 4);
    }

    #[test]
    fn a_dropped_reservation_leaves_the_list_untouched() {
        let mut fl = FreeList::new((4..8).map(PhysReg::from_index));

        assert!(fl.reserve(2).is_some());

        assert_eq!(fl.available(), 4);
    }

    #[test]
    fn a_vector_free_list_allocates_in_order_and_reclaims() {
        let mut fl = FreeList::new((32..64).map(VecPhysReg::from_index));
        assert_eq!(fl.available(), 32);

        let p = fl.allocate().unwrap();
        assert_eq!(p, VecPhysReg::from_index(32));
        assert_eq!(fl.available(), 31);

        fl.reclaim(p);
        assert_eq!(fl.available(), 32);
    }

    #[test]
    fn vector_register_zero_is_reclaimed_like_any_other() {
        let mut fl = FreeList::new((4..8).map(VecPhysReg::from_index));
        let initial = fl.available();

        fl.reclaim(VecPhysReg::ZERO);

        assert_eq!(fl.available(), initial + 1);
    }
}

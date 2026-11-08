//! Load-reserved / store-conditional reservations for every hart.
//!
//! A reservation is a cache-line-aligned physical address held by one hart
//! between an LR and its SC. Reservations live in shared state rather than
//! on the hart because a store from *any* hart to the reserved line must
//! break it: that is what makes LR/SC spinlocks work across cores.

use super::write_log::Writer;
use crate::common::{HartId, PhysAddr, PhysRange};

/// Cache line size for reservation granularity; the SC-success granule is
/// implementation-defined and 64 bytes matches the L1D line.
const RESERVATION_GRANULE: u64 = 64;

/// One reservation slot per hart.
#[derive(Clone, Debug)]
pub struct ReservationSet {
    slots: Vec<Option<PhysAddr>>,
}

impl ReservationSet {
    /// Empty reservations for `hart_count` harts.
    #[must_use]
    pub fn new(hart_count: usize) -> Self {
        Self { slots: vec![None; hart_count] }
    }

    const fn align(addr: PhysAddr) -> PhysAddr {
        PhysAddr::new(addr.val() & !(RESERVATION_GRANULE - 1))
    }

    /// Reserves the line containing `addr` for `hart`.
    pub fn set(&mut self, hart: HartId, addr: PhysAddr) {
        self.slots[hart.as_index()] = Some(Self::align(addr));
    }

    /// True when `hart` holds a reservation covering `addr`.
    #[must_use]
    pub fn check(&self, hart: HartId, addr: PhysAddr) -> bool {
        self.slots[hart.as_index()] == Some(Self::align(addr))
    }

    /// The line `hart` holds reserved, if any.
    #[must_use]
    pub fn reserved(&self, hart: HartId) -> Option<PhysAddr> {
        self.slots[hart.as_index()]
    }

    /// Drops `hart`'s reservation.
    pub fn clear(&mut self, hart: HartId) {
        self.slots[hart.as_index()] = None;
    }

    /// Breaks the reservation of every hart but `writer` on a granule the
    /// bytes `written` touch, as a write does.
    pub fn break_on_write(&mut self, writer: Writer, written: PhysRange) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let is_writer = matches!(writer, Writer::Hart(hart) if hart.as_index() == index);
            let is_touched =
                slot.is_some_and(|granule| written.overlaps(granule, RESERVATION_GRANULE));
            if is_touched && !is_writer {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_from_another_hart_breaks_the_reservation() {
        let mut set = ReservationSet::new(2);
        let h0 = HartId::new(0);
        let h1 = HartId::new(1);
        set.set(h0, PhysAddr::new(0x1008));
        assert!(set.check(h0, PhysAddr::new(0x1000)));
        assert!(!set.check(h0, PhysAddr::new(0x1040)));
        set.break_on_write(Writer::Hart(h0), PhysRange::new(PhysAddr::new(0x1010), 8));
        assert!(set.check(h0, PhysAddr::new(0x1000)), "the writer keeps its own reservation");
        set.break_on_write(Writer::Hart(h1), PhysRange::new(PhysAddr::new(0x1010), 8));
        assert!(!set.check(h0, PhysAddr::new(0x1000)));
        set.set(h1, PhysAddr::new(0x2000));
        set.clear(h1);
        assert!(!set.check(h1, PhysAddr::new(0x2000)));
    }
}

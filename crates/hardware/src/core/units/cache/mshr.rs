//! Miss Status Holding Registers for the event-driven cache.
//!
//! One MSHR per line being fetched from the next level. Requests that miss
//! on a line already in flight join its MSHR as targets instead of sending
//! a second request downstream; when the fill arrives every target is
//! answered. A prefetch MSHR starts with no targets, and a demand request
//! that joins it later is what makes the prefetch useful.

use crate::common::{LineAddr, PhysAddr, VirtAddr};
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, MemOp};

/// A request waiting for an MSHR's fill.
#[derive(Clone, Debug)]
pub struct MshrTarget {
    /// Component that issued the request.
    pub source: ComponentId,
    /// The requester's correlator, echoed on the response.
    pub req_id: ReqId,
    /// Requested address.
    pub paddr: PhysAddr,
    /// Pre-translation address for fault reporting.
    pub vaddr: Option<VirtAddr>,
    /// Access width.
    pub size: AccessSize,
    /// Read / write / atomic / fetch.
    pub op: MemOp,
}

/// An outstanding line fetch.
#[derive(Clone, Debug)]
pub struct Mshr {
    /// Line being fetched.
    pub line: LineAddr,
    /// Correlator of the request this cache sent downstream.
    pub req_id: ReqId,
    /// Requests waiting for the line, in arrival order.
    pub targets: Vec<MshrTarget>,
    /// Requests the fill may not serve: writes that joined after a request
    /// without write permission was sent, and everything that arrived after
    /// them (gem5's deferred targets). Served once the line is writable.
    pub deferred: Vec<MshrTarget>,
    /// True when the downstream request asked for write permission, so the
    /// fill installs the line dirty.
    pub write: bool,
    /// True when the fetch was started by the prefetcher rather than a
    /// demand request.
    pub prefetch: bool,
    /// Cycle the downstream request was sent.
    pub issued_at: u64,
    /// True when the line was already held and only write permission was
    /// requested.
    pub upgrade: bool,
}

/// Bounded table of outstanding line fetches.
#[derive(Debug)]
pub struct MshrTable {
    entries: Vec<Mshr>,
    capacity: usize,
}

impl MshrTable {
    /// A table with room for `capacity` outstanding lines; zero is treated
    /// as one (a blocking cache still needs one miss in flight).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self { entries: Vec::with_capacity(capacity), capacity }
    }

    /// Room for `capacity` outstanding lines.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Outstanding lines.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no line is outstanding.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when no more lines can be fetched until a fill returns.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.entries.len() >= self.capacity
    }

    /// Free slots.
    #[must_use]
    pub const fn free(&self) -> usize {
        self.capacity - self.entries.len()
    }

    /// The MSHR fetching `line`, if any.
    #[must_use]
    pub fn find_line_mut(&mut self, line: LineAddr) -> Option<&mut Mshr> {
        self.entries.iter_mut().find(|m| m.line == line)
    }

    /// True while `line` is being fetched.
    #[must_use]
    pub fn holds(&self, line: LineAddr) -> bool {
        self.entries.iter().any(|m| m.line == line)
    }

    /// Records a new outstanding fetch. The caller must have checked
    /// [`MshrTable::is_full`].
    pub fn allocate(&mut self, mshr: Mshr) {
        debug_assert!(!self.is_full(), "MSHR allocation on a full table");
        debug_assert!(!self.holds(mshr.line), "duplicate MSHR for a line");
        self.entries.push(mshr);
    }

    /// Removes and returns the MSHR whose downstream request was `req_id`.
    pub fn take(&mut self, req_id: ReqId) -> Option<Mshr> {
        let index = self.entries.iter().position(|m| m.req_id == req_id)?;
        Some(self.entries.remove(index))
    }

    /// Outstanding fetches, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Mshr> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::components::CacheId;

    fn mshr(line: u64, req: u64) -> Mshr {
        Mshr {
            line: LineAddr::from_phys(PhysAddr::new(line), 64),
            req_id: ReqId::new(req),
            targets: Vec::new(),
            deferred: Vec::new(),
            write: false,
            prefetch: false,
            issued_at: 0,
            upgrade: false,
        }
    }

    #[test]
    fn allocation_is_bounded_and_take_frees_a_slot() {
        let mut table = MshrTable::new(2);
        table.allocate(mshr(0x1000, 1));
        table.allocate(mshr(0x2000, 2));
        assert!(table.is_full());
        assert_eq!(table.free(), 0);
        assert!(table.holds(LineAddr::from_phys(PhysAddr::new(0x1010), 64)));

        let taken = table.take(ReqId::new(1)).expect("req 1 outstanding");
        assert_eq!(taken.line, LineAddr::from_phys(PhysAddr::new(0x1000), 64));
        assert_eq!(table.free(), 1);
        assert!(table.take(ReqId::new(1)).is_none());
    }

    #[test]
    fn zero_capacity_still_allows_one_miss() {
        let table = MshrTable::new(0);
        assert_eq!(table.capacity(), 1);
        assert!(!table.is_full());
    }

    #[test]
    fn targets_join_the_mshr_for_their_line() {
        let mut table = MshrTable::new(2);
        table.allocate(mshr(0x1000, 1));
        let entry = table
            .find_line_mut(LineAddr::from_phys(PhysAddr::new(0x1008), 64))
            .expect("line in flight");
        entry.targets.push(MshrTarget {
            source: ComponentId::Cache(CacheId::new(0)),
            req_id: ReqId::new(9),
            paddr: PhysAddr::new(0x1008),
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Write { data: crate::sim::packet::WriteData::Small(1) },
        });
        entry.write = true;
        assert_eq!(table.iter().next().map(|m| m.targets.len()), Some(1));
    }
}

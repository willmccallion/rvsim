//! Writeback buffer: dirty (or, for an exclusive pair, clean) victims on
//! their way to the next level.
//!
//! An entry is allocated when a line leaves the cache and freed when the
//! next level acknowledges the writeback. While the buffer is at capacity
//! the cache blocks new requests, as gem5's `Blocked_NoWBBuffers` does.

use crate::common::LineAddr;
use crate::sim::components::ReqId;

/// One writeback in flight.
#[derive(Clone, Copy, Debug)]
pub struct Writeback {
    /// Line being written back.
    pub line: LineAddr,
    /// Correlator of the request sent downstream.
    pub req_id: ReqId,
}

/// Bounded set of writebacks awaiting the next level's acknowledgement.
#[derive(Debug)]
pub struct WritebackBuffer {
    entries: Vec<Writeback>,
    capacity: usize,
}

impl WritebackBuffer {
    /// A buffer that blocks the cache once `capacity` writebacks are in
    /// flight; zero is treated as one.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self { entries: Vec::with_capacity(capacity), capacity }
    }

    #[cfg(test)]
    /// Writebacks in flight.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    /// True when nothing is in flight.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when the cache must stop accepting requests. Fills already in
    /// progress may still push the buffer past this point; that is the
    /// condition that blocks the request port.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.entries.len() >= self.capacity
    }

    /// Records a writeback that has been sent downstream.
    pub fn allocate(&mut self, writeback: Writeback) {
        self.entries.push(writeback);
    }

    /// Frees the writeback acknowledged by `req_id`; false if none matches.
    pub fn complete(&mut self, req_id: ReqId) -> bool {
        match self.entries.iter().position(|w| w.req_id == req_id) {
            Some(index) => {
                let _ = self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    /// True while `line` is being written back.
    #[must_use]
    pub fn holds(&self, line: LineAddr) -> bool {
        self.entries.iter().any(|w| w.line == line)
    }

    /// Lines in flight.
    pub fn lines(&self) -> impl Iterator<Item = LineAddr> + '_ {
        self.entries.iter().map(|w| w.line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::PhysAddr;

    #[test]
    fn completion_frees_the_matching_entry_only() {
        let mut buffer = WritebackBuffer::new(1);
        let line = LineAddr::from_phys(PhysAddr::new(0x1000), 64);
        buffer.allocate(Writeback { line, req_id: ReqId::new(5) });
        assert!(buffer.is_full());
        assert!(buffer.holds(line));
        assert!(!buffer.complete(ReqId::new(6)));
        assert!(buffer.complete(ReqId::new(5)));
        assert!(buffer.is_empty());
    }
}

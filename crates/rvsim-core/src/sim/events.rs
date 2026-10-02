//! Global event queue and `Event` record.
//!
//! All inter-component traffic on the simulated chip travels as scheduled
//! events. Components do not call each other directly; they schedule a packet
//! delivery at some future cycle via [`EventQueue::schedule`]. The simulator's
//! main loop drains the queue at the start of every cycle and dispatches each
//! ready event to its target's [`Handle`](crate::sim::handle::Handle) impl.
//!
//! Ordering is deterministic: events sort by `(fire_at, seq)` where `seq` is a
//! monotonically increasing tiebreaker assigned at schedule time. Same inputs
//! + same seed + same config produce the same trace.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::sim::components::ComponentId;
use crate::sim::packet::Packet;

/// One scheduled message between two components.
#[derive(Clone, Debug)]
pub struct Event {
    /// Cycle at which this event becomes deliverable.
    pub fire_at: u64,
    /// Monotonic tiebreaker for events scheduled at the same cycle.
    pub seq: u64,
    /// Component that receives the packet.
    pub target: ComponentId,
    /// Component that scheduled the packet (for response routing).
    pub source: ComponentId,
    /// Payload.
    pub packet: Packet,
}

impl Event {
    #[inline]
    const fn ordering_key(&self) -> (u64, u64) {
        (self.fire_at, self.seq)
    }
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.ordering_key() == other.ordering_key()
    }
}

impl Eq for Event {}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Event {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ordering_key().cmp(&other.ordering_key())
    }
}

/// Min-heap of `Event`s keyed on `(fire_at, seq)`.
#[derive(Debug, Default)]
pub struct EventQueue {
    heap: BinaryHeap<Reverse<Event>>,
    next_seq: u64,
}

impl EventQueue {
    /// Constructs an empty queue.
    #[inline]
    pub const fn new() -> Self {
        Self { heap: BinaryHeap::new(), next_seq: 0 }
    }

    /// Schedules a packet for delivery at `fire_at`. The assigned sequence number
    /// breaks ties between events scheduled at the same cycle in FIFO order.
    pub fn schedule(
        &mut self,
        fire_at: u64,
        target: ComponentId,
        source: ComponentId,
        packet: Packet,
    ) {
        let event = Event { fire_at, seq: self.next_seq, target, source, packet };
        self.next_seq += 1;
        self.heap.push(Reverse(event));
    }

    /// Pops the next event if its `fire_at <= cycle`; otherwise leaves the heap
    /// untouched and returns `None`.
    pub fn pop_ready(&mut self, cycle: u64) -> Option<Event> {
        match self.heap.peek() {
            Some(Reverse(top)) if top.fire_at <= cycle => self.heap.pop().map(|Reverse(e)| e),
            _ => None,
        }
    }

    /// True when the queue holds no events.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{LineAddr, PhysAddr};
    use crate::sim::components::{CacheId, PipelineId, ReqId};
    use crate::sim::packet::{HitLevel, MemRespData, MesiState};

    fn make_resp(req_id: u64) -> Packet {
        Packet::MemResp {
            req_id: ReqId::new(req_id),
            line_addr: LineAddr::from_phys(PhysAddr::new(0), 64),
            data: MemRespData::Small(0),
            hit_level: HitLevel::L1,
            state: MesiState::Exclusive,
        }
    }

    #[test]
    fn pop_ready_respects_cycle() {
        let mut q = EventQueue::new();
        let src = ComponentId::Pipeline(PipelineId::new(0));
        let dst = ComponentId::Cache(CacheId::new(0));
        q.schedule(10, dst, src, make_resp(1));
        assert!(q.pop_ready(5).is_none());
        let evt = q.pop_ready(10).expect("event ready at cycle 10");
        assert_eq!(evt.fire_at, 10);
    }

    #[test]
    fn events_ordered_by_fire_at_then_seq() {
        let mut q = EventQueue::new();
        let src = ComponentId::Pipeline(PipelineId::new(0));
        let dst = ComponentId::Cache(CacheId::new(0));
        // Out-of-order schedule; same fire_at for two of them.
        q.schedule(20, dst, src, make_resp(20));
        q.schedule(10, dst, src, make_resp(10));
        q.schedule(10, dst, src, make_resp(11));

        let a = q.pop_ready(100).unwrap();
        assert_eq!(a.fire_at, 10);
        match a.packet {
            Packet::MemResp { req_id, .. } => assert_eq!(req_id, ReqId::new(10)),
            _ => panic!("wrong packet"),
        }
        let b = q.pop_ready(100).unwrap();
        assert_eq!(b.fire_at, 10);
        match b.packet {
            Packet::MemResp { req_id, .. } => assert_eq!(req_id, ReqId::new(11)),
            _ => panic!("wrong packet"),
        }
        let c = q.pop_ready(100).unwrap();
        assert_eq!(c.fire_at, 20);
        assert!(q.is_empty());
    }
}

//! Interconnects: the timing of moving a coherence message from one node
//! to another.
//!
//! An interconnect accepts messages from nodes, advances once per cycle,
//! and hands over the messages that arrive that cycle. Every
//! implementation keeps one queue per virtual channel ([`MsgClass`]) so a
//! stalled request can never block a response behind it, and arbitrates
//! deterministically (oldest message first, then lowest port).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use super::messages::{CoherenceMsg, MsgClass, Node};
use super::stats::InterconnectStatPaths;
use crate::common::CoreId;
use crate::sim::stats::Stats;

/// What a topology looks like, for stats and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyInfo {
    /// Human-readable kind (`crossbar`, `ring`, ...).
    pub kind: &'static str,
    /// Endpoints, including the home agent.
    pub endpoints: usize,
    /// Routers or switch ports.
    pub nodes: usize,
    /// Longest hop count between two endpoints.
    pub diameter: usize,
}

/// A message-moving fabric between requesting agents and the home.
pub trait Interconnect: Send + Sync + std::fmt::Debug {
    /// Injects `msg` at `from` at cycle `now`.
    fn send(&mut self, now: u64, from: Node, msg: CoherenceMsg);

    /// Advances one cycle and hands every message arriving at `now` to
    /// `deliver` with its destination.
    fn tick(&mut self, now: u64, stats: &mut Stats, deliver: &mut dyn FnMut(Node, CoherenceMsg));

    /// True while any message is in flight.
    fn is_idle(&self) -> bool;

    /// Shape of the network.
    fn topology(&self) -> TopologyInfo;
}

/// Where the home agent sits on a fabric whose ports are numbered: cores
/// take ports `0..n`, the home takes port `n`.
#[must_use]
pub const fn port_of(node: Node, cores: usize) -> usize {
    match node {
        Node::Core(core) => core.as_index(),
        Node::Home => cores,
    }
}

/// The node on port `port`.
#[must_use]
pub fn node_of(port: usize, cores: usize) -> Node {
    if port == cores { Node::Home } else { Node::Core(CoreId::new(u32::try_from(port).unwrap_or(u32::MAX))) }
}

/// A message waiting in an input queue.
#[derive(Clone, Copy, Debug)]
struct Queued {
    msg: CoherenceMsg,
    arrived: u64,
}

/// A message in transit to its destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InFlight {
    deliver_at: u64,
    seq: u64,
    to: Node,
    msg: CoherenceMsg,
}

impl PartialOrd for InFlight {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InFlight {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.deliver_at, self.seq).cmp(&(other.deliver_at, other.seq))
    }
}

/// A full crossbar: any input can reach any output; each output accepts
/// one message per class per transfer time, and a transfer takes
/// `hop_latency` plus `ceil(bytes / bytes_per_cycle)` cycles.
#[derive(Debug)]
pub struct Crossbar {
    cores: usize,
    line_bytes: usize,
    hop_latency: u64,
    bytes_per_cycle: usize,
    /// Input queues indexed `[port][class]`.
    inputs: Vec<[VecDeque<Queued>; 4]>,
    /// Cycle each `(output port, class)` becomes free.
    output_free_at: Vec<[u64; 4]>,
    /// Round-robin start per output port.
    rr: Vec<usize>,
    in_flight: BinaryHeap<Reverse<InFlight>>,
    next_seq: u64,
    stat_paths: InterconnectStatPaths,
}

impl Crossbar {
    /// A crossbar for `cores` requesting agents plus the home.
    #[must_use]
    pub fn new(cores: usize, line_bytes: usize, hop_latency: u64, bytes_per_cycle: usize, stat_paths: InterconnectStatPaths) -> Self {
        let ports = cores + 1;
        Self {
            cores,
            line_bytes,
            hop_latency,
            bytes_per_cycle: bytes_per_cycle.max(1),
            inputs: (0..ports).map(|_| std::array::from_fn(|_| VecDeque::new())).collect(),
            output_free_at: vec![[0; 4]; ports],
            rr: vec![0; ports],
            in_flight: BinaryHeap::new(),
            next_seq: 0,
            stat_paths,
        }
    }

    const fn transfer_cycles(&self, msg: CoherenceMsg) -> u64 {
        (msg.bytes(self.line_bytes).div_ceil(self.bytes_per_cycle)) as u64
    }

    /// One arbitration pass: for every free `(output, class)` pick the
    /// oldest waiting message across inputs (ties by port, rotating).
    fn arbitrate(&mut self, now: u64, stats: &mut Stats) {
        let ports = self.inputs.len();
        for out in 0..ports {
            for class in MsgClass::ALL {
                let c = class.index();
                if self.output_free_at[out][c] > now {
                    stats.counter(self.stat_paths.busy_cycles).inc();
                    continue;
                }
                let mut best: Option<(u64, usize)> = None;
                for offset in 0..ports {
                    let port = (self.rr[out] + offset) % ports;
                    if let Some(head) = self.inputs[port][c].front()
                        && port_of(head.msg.destination(), self.cores) == out
                        && best.is_none_or(|(arrived, _)| head.arrived < arrived)
                    {
                        best = Some((head.arrived, port));
                    }
                }
                let Some((_, port)) = best else { continue };
                let Some(queued) = self.inputs[port][c].pop_front() else { continue };
                if queued.arrived < now {
                    stats.counter(self.stat_paths.blocked_cycles).add(now - queued.arrived);
                }
                let transfer = self.transfer_cycles(queued.msg);
                self.output_free_at[out][c] = now + transfer;
                self.rr[out] = (port + 1) % ports;
                stats.counter(self.stat_paths.messages).inc();
                stats.counter(self.stat_paths.bytes).add(queued.msg.bytes(self.line_bytes) as u64);
                let seq = self.next_seq;
                self.next_seq += 1;
                self.in_flight.push(Reverse(InFlight {
                    deliver_at: now + self.hop_latency + transfer,
                    seq,
                    to: queued.msg.destination(),
                    msg: queued.msg,
                }));
            }
        }
    }
}

impl Interconnect for Crossbar {
    fn send(&mut self, now: u64, from: Node, msg: CoherenceMsg) {
        let port = port_of(from, self.cores);
        self.inputs[port][msg.class().index()].push_back(Queued { msg, arrived: now });
    }

    fn tick(&mut self, now: u64, stats: &mut Stats, deliver: &mut dyn FnMut(Node, CoherenceMsg)) {
        self.arbitrate(now, stats);
        while let Some(Reverse(next)) = self.in_flight.peek() {
            if next.deliver_at > now {
                break;
            }
            let Some(Reverse(flight)) = self.in_flight.pop() else { break };
            deliver(flight.to, flight.msg);
        }
    }

    fn is_idle(&self) -> bool {
        self.in_flight.is_empty() && self.inputs.iter().all(|queues| queues.iter().all(VecDeque::is_empty))
    }

    fn topology(&self) -> TopologyInfo {
        TopologyInfo { kind: "crossbar", endpoints: self.cores + 1, nodes: self.cores + 1, diameter: 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coherence::messages::ReqKind;
    use crate::common::{LineAddr, PhysAddr};
    use crate::sim::components::ReqId;
    use crate::sim::packet::MesiState;

    fn paths() -> InterconnectStatPaths {
        InterconnectStatPaths::new("test.xbar")
    }

    fn req(txn: u64, core: u32) -> CoherenceMsg {
        CoherenceMsg::Req {
            txn: ReqId::new(txn),
            line: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            kind: ReqKind::ReadShared,
            requester: CoreId::new(core),
        }
    }

    fn data(txn: u64, core: u32) -> CoherenceMsg {
        CoherenceMsg::CompData {
            txn: ReqId::new(txn),
            line: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            to: CoreId::new(core),
            state: MesiState::Shared,
        }
    }

    fn run(xbar: &mut Crossbar, from: u64, to: u64) -> Vec<(u64, Node, CoherenceMsg)> {
        let mut stats = Stats::new();
        let mut out = Vec::new();
        for now in from..=to {
            xbar.tick(now, &mut stats, &mut |node, msg| out.push((now, node, msg)));
        }
        out
    }

    #[test]
    fn a_message_arrives_after_hop_latency_plus_its_transfer_time() {
        let mut xbar = Crossbar::new(2, 64, 3, 16, paths());
        xbar.send(10, Node::Core(CoreId::new(0)), req(1, 0));
        xbar.send(10, Node::Home, data(2, 1));
        let delivered = run(&mut xbar, 10, 30);
        assert_eq!(delivered[0].0, 10 + 3 + 1, "8-byte request: one transfer cycle");
        assert_eq!(delivered[0].1, Node::Home);
        assert_eq!(delivered[1].0, 10 + 3 + 5, "72-byte data at 16 B/cycle: five transfer cycles");
        assert_eq!(delivered[1].1, Node::Core(CoreId::new(1)));
        assert!(xbar.is_idle());
    }

    #[test]
    fn one_output_serialises_messages_of_a_class_oldest_first() {
        let mut xbar = Crossbar::new(2, 64, 1, 8, paths());
        xbar.send(5, Node::Core(CoreId::new(1)), req(1, 1));
        xbar.send(4, Node::Core(CoreId::new(0)), req(2, 0));
        let delivered = run(&mut xbar, 5, 20);
        let order: Vec<u64> = delivered.iter().map(|(_, _, m)| m.txn().val()).collect();
        assert_eq!(order, vec![2, 1], "the request that waited longer goes first");
        assert_eq!(delivered[0].0, 5 + 1 + 1);
        assert_eq!(delivered[1].0, 6 + 1 + 1, "the second transfer starts when the port frees");
    }

    #[test]
    fn classes_do_not_block_each_other() {
        let mut xbar = Crossbar::new(1, 64, 1, 8, paths());
        // A long data transfer to core 0 and a short response to core 0 in the same cycle.
        xbar.send(0, Node::Home, data(1, 0));
        xbar.send(0, Node::Home, CoherenceMsg::Comp { txn: ReqId::new(2), line: LineAddr::from_phys(PhysAddr::new(0), 64), to: CoreId::new(0), state: MesiState::Modified });
        let delivered = run(&mut xbar, 0, 20);
        let comp_at = delivered.iter().find(|(_, _, m)| m.txn().val() == 2).map(|d| d.0);
        assert_eq!(comp_at, Some(2), "the response channel is not behind the data channel");
    }
}

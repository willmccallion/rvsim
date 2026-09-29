//! A network that routes messages hop by hop over a topology.

use super::{InFlight, NetworkTopology, TopologyInfo, node_of, port_of};
use crate::sim::packet::coherence::{CoherenceMsg, MsgClass, Node};
use crate::sim::stats::Stats;
use crate::soc::coherence::Interconnect;
use crate::soc::coherence::stats::InterconnectStatPaths;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

/// A message travelling a routed network.
#[derive(Clone, Copy, Debug)]
pub(super) struct Routed {
    msg: CoherenceMsg,
    to_node: usize,
    arrived: u64,
}

/// A network of routers with per-link, per-class queues over a
/// [`NetworkTopology`]: a message crosses one link per `hop_latency` plus
/// transfer time, and each link moves one message per class at a time.
#[derive(Debug)]
pub struct RoutedNetwork<T: NetworkTopology> {
    topology: T,
    cores: usize,
    line_bytes: usize,
    hop_latency: u64,
    bytes_per_cycle: usize,
    /// Output queues indexed `[node][neighbour index][class]`.
    queues: Vec<Vec<[VecDeque<Routed>; 4]>>,
    /// Cycle each `[node][neighbour index][class]` link frees.
    link_free_at: Vec<Vec<[u64; 4]>>,
    in_flight: BinaryHeap<Reverse<InFlight>>,
    next_seq: u64,
    stat_paths: InterconnectStatPaths,
}

impl<T: NetworkTopology> RoutedNetwork<T> {
    /// A network for `cores` requesting agents plus the home on
    /// `topology`.
    ///
    /// # Panics
    ///
    /// Panics when `topology` has fewer than `cores + 1` nodes.
    #[must_use]
    pub fn new(
        topology: T,
        cores: usize,
        line_bytes: usize,
        hop_latency: u64,
        bytes_per_cycle: usize,
        stat_paths: InterconnectStatPaths,
    ) -> Self {
        let nodes = topology.node_count();
        assert!(nodes > cores, "topology must have a node per core plus the home");
        let queues = (0..nodes)
            .map(|node| {
                (0..topology.neighbours(node).len())
                    .map(|_| std::array::from_fn(|_| VecDeque::new()))
                    .collect()
            })
            .collect();
        let link_free_at =
            (0..nodes).map(|node| vec![[0; 4]; topology.neighbours(node).len()]).collect();
        Self {
            topology,
            cores,
            line_bytes,
            hop_latency,
            bytes_per_cycle: bytes_per_cycle.max(1),
            queues,
            link_free_at,
            in_flight: BinaryHeap::new(),
            next_seq: 0,
            stat_paths,
        }
    }

    const fn transfer_cycles(&self, msg: CoherenceMsg) -> u64 {
        (msg.bytes(self.line_bytes).div_ceil(self.bytes_per_cycle)) as u64
    }

    /// Queues `routed` at `node` on the link towards its destination.
    fn enqueue(&mut self, node: usize, routed: Routed) {
        let next = self.topology.next_hop(node, routed.to_node);
        let link =
            self.topology.neighbours(node).iter().position(|n| *n == next).unwrap_or_default();
        self.queues[node][link][routed.msg.class().index()].push_back(routed);
    }

    fn forward(&mut self, now: u64, stats: &mut Stats) {
        for node in 0..self.queues.len() {
            let neighbours = self.topology.neighbours(node);
            for (link, &next) in neighbours.iter().enumerate() {
                for class in MsgClass::ALL {
                    let c = class.index();
                    if self.link_free_at[node][link][c] > now {
                        continue;
                    }
                    let Some(routed) = self.queues[node][link][c].pop_front() else { continue };
                    if routed.arrived < now {
                        stats.counter(self.stat_paths.blocked_cycles).add(now - routed.arrived);
                    }
                    let transfer = self.transfer_cycles(routed.msg);
                    self.link_free_at[node][link][c] = now + transfer;
                    stats.counter(self.stat_paths.busy_cycles).add(transfer);
                    let seq = self.next_seq;
                    self.next_seq += 1;
                    self.in_flight.push(Reverse(InFlight {
                        deliver_at: now + self.hop_latency + transfer,
                        seq,
                        to: node_of(next, self.cores),
                        msg: routed.msg,
                    }));
                }
            }
        }
    }
}

impl<T: NetworkTopology> Interconnect for RoutedNetwork<T> {
    fn send(&mut self, now: u64, from: Node, msg: CoherenceMsg) {
        let to_node = port_of(msg.destination(), self.cores);
        let from_node = port_of(from, self.cores);
        self.enqueue(from_node, Routed { msg, to_node, arrived: now });
    }

    fn tick(&mut self, now: u64, stats: &mut Stats, deliver: &mut dyn FnMut(Node, CoherenceMsg)) {
        while let Some(Reverse(next)) = self.in_flight.peek() {
            if next.deliver_at > now {
                break;
            }
            let Some(Reverse(flight)) = self.in_flight.pop() else { break };
            let at_node = port_of(flight.to, self.cores);
            let to_node = port_of(flight.msg.destination(), self.cores);
            if at_node == to_node {
                stats.counter(self.stat_paths.messages).inc();
                stats.counter(self.stat_paths.bytes).add(flight.msg.bytes(self.line_bytes) as u64);
                deliver(flight.msg.destination(), flight.msg);
            } else {
                self.enqueue(at_node, Routed { msg: flight.msg, to_node, arrived: now });
            }
        }
        self.forward(now, stats);
    }

    fn is_idle(&self) -> bool {
        self.in_flight.is_empty()
            && self
                .queues
                .iter()
                .all(|links| links.iter().all(|qs| qs.iter().all(VecDeque::is_empty)))
    }

    fn topology(&self) -> TopologyInfo {
        TopologyInfo {
            kind: self.topology.kind(),
            endpoints: self.cores + 1,
            nodes: self.topology.node_count(),
            diameter: self.topology.diameter(),
        }
    }
}

#[cfg(test)]
mod routed_tests {
    use super::*;
    use crate::common::CoreId;
    use crate::common::{LineAddr, PhysAddr};
    use crate::sim::components::ReqId;
    use crate::sim::packet::coherence::ReqKind;
    use crate::soc::coherence::interconnect::{Hypercube, Mesh2D, Ring};

    fn req(core: u32) -> CoherenceMsg {
        CoherenceMsg::Req {
            txn: ReqId::new(u64::from(core)),
            line: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            kind: ReqKind::ReadShared,
            requester: CoreId::new(core),
        }
    }

    fn deliver_all(net: &mut dyn Interconnect, from: u64, to: u64) -> Vec<(u64, Node)> {
        let mut stats = Stats::new();
        let mut out = Vec::new();
        for now in from..=to {
            net.tick(now, &mut stats, &mut |node, _| out.push((now, node)));
        }
        out
    }

    #[test]
    fn ring_routes_take_the_shorter_direction() {
        let ring = Ring::new(6);
        assert_eq!(ring.next_hop(0, 2), 1);
        assert_eq!(ring.next_hop(0, 5), 5);
        assert_eq!(ring.next_hop(0, 3), 1, "tie goes forward");
        assert_eq!(ring.diameter(), 3);
    }

    #[test]
    fn mesh_routes_x_first_and_torus_wraps() {
        let mesh = Mesh2D::for_endpoints(5, false);
        assert_eq!(mesh.node_count(), 9);
        assert_eq!(mesh.next_hop(0, 8), 1, "move along x first");
        assert_eq!(mesh.next_hop(2, 8), 5, "then along y");
        assert_eq!(mesh.neighbours(4), vec![1, 3, 5, 7]);
        let torus = Mesh2D::for_endpoints(5, true);
        assert_eq!(torus.next_hop(0, 2), 2, "wraparound is one hop");
        assert_eq!(torus.diameter(), 2);
    }

    #[test]
    fn hypercube_flips_one_dimension_per_hop() {
        let cube = Hypercube::for_endpoints(5);
        assert_eq!(cube.node_count(), 8);
        assert_eq!(cube.neighbours(0), vec![1, 2, 4]);
        assert_eq!(cube.next_hop(0, 7), 1);
        assert_eq!(cube.next_hop(1, 7), 3);
        assert_eq!(cube.next_hop(3, 7), 7);
        assert_eq!(cube.diameter(), 3);
    }

    #[test]
    fn a_message_pays_hop_latency_per_hop() {
        // 4 cores + home on a 6-stop ring: core 0 at node 0, home at node 4.
        let mut net =
            RoutedNetwork::new(Ring::new(6), 4, 64, 2, 8, InterconnectStatPaths::new("t"));
        net.send(0, Node::Core(CoreId::new(0)), req(0));
        let delivered = deliver_all(&mut net, 0, 40);
        // Two hops backwards (0 -> 5 -> 4), each 2 + 1 cycles.
        assert_eq!(delivered, vec![(6, Node::Home)]);
        assert!(net.is_idle());
    }

    #[test]
    fn a_busy_link_serialises_same_class_messages() {
        let mut net =
            RoutedNetwork::new(Ring::new(3), 2, 64, 1, 8, InterconnectStatPaths::new("t"));
        net.send(0, Node::Core(CoreId::new(0)), req(0));
        net.send(0, Node::Core(CoreId::new(0)), req(0));
        let delivered = deliver_all(&mut net, 0, 20);
        assert_eq!(delivered.iter().map(|d| d.0).collect::<Vec<_>>(), vec![2, 3]);
    }
}

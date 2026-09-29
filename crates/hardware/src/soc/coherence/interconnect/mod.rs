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

use super::stats::InterconnectStatPaths;
use crate::common::CoreId;
use crate::sim::packet::coherence::{CoherenceMsg, MsgClass, Node};
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

    /// True when idle and ticking at cycle `now` or later changes nothing.
    fn is_quiet(&self, _now: u64) -> bool {
        self.is_idle()
    }

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
    if port == cores {
        Node::Home
    } else {
        Node::Core(CoreId::new(u32::try_from(port).unwrap_or(u32::MAX)))
    }
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
    pub fn new(
        cores: usize,
        line_bytes: usize,
        hop_latency: u64,
        bytes_per_cycle: usize,
        stat_paths: InterconnectStatPaths,
    ) -> Self {
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
        self.in_flight.is_empty()
            && self.inputs.iter().all(|queues| queues.iter().all(VecDeque::is_empty))
    }

    /// An output still transmitting counts a busy cycle every tick.
    fn is_quiet(&self, now: u64) -> bool {
        self.is_idle() && self.output_free_at.iter().flatten().all(|&free_at| free_at <= now)
    }

    fn topology(&self) -> TopologyInfo {
        TopologyInfo {
            kind: "crossbar",
            endpoints: self.cores + 1,
            nodes: self.cores + 1,
            diameter: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{LineAddr, PhysAddr};
    use crate::sim::components::ReqId;
    use crate::sim::packet::MesiState;
    use crate::sim::packet::coherence::ReqKind;

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
        xbar.send(
            0,
            Node::Home,
            CoherenceMsg::Comp {
                txn: ReqId::new(2),
                line: LineAddr::from_phys(PhysAddr::new(0), 64),
                to: CoreId::new(0),
                state: MesiState::Modified,
            },
        );
        let delivered = run(&mut xbar, 0, 20);
        let comp_at = delivered.iter().find(|(_, _, m)| m.txn().val() == 2).map(|d| d.0);
        assert_eq!(comp_at, Some(2), "the response channel is not behind the data channel");
    }
}

/// The shape of a routed network: which nodes exist, who neighbours whom,
/// and the next hop from any node towards any other.
pub trait NetworkTopology: Send + Sync + std::fmt::Debug {
    /// Human-readable kind.
    fn kind(&self) -> &'static str;
    /// Routers in the network (at least the number of endpoints).
    fn node_count(&self) -> usize;
    /// Neighbours of `node`, in a fixed order.
    fn neighbours(&self, node: usize) -> Vec<usize>;
    /// Next node on the route from `from` to `to` (`from != to`).
    fn next_hop(&self, from: usize, to: usize) -> usize;
    /// Longest route between two nodes, in hops.
    fn diameter(&self) -> usize;
}

/// A bidirectional ring; routes take the shorter direction.
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    nodes: usize,
}

impl Ring {
    /// A ring of `nodes` stops (at least two).
    #[must_use]
    pub fn new(nodes: usize) -> Self {
        Self { nodes: nodes.max(2) }
    }
}

impl NetworkTopology for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn node_count(&self) -> usize {
        self.nodes
    }

    fn neighbours(&self, node: usize) -> Vec<usize> {
        vec![(node + self.nodes - 1) % self.nodes, (node + 1) % self.nodes]
    }

    fn next_hop(&self, from: usize, to: usize) -> usize {
        let forward = (to + self.nodes - from) % self.nodes;
        if forward <= self.nodes - forward {
            (from + 1) % self.nodes
        } else {
            (from + self.nodes - 1) % self.nodes
        }
    }

    fn diameter(&self) -> usize {
        self.nodes / 2
    }
}

/// A `k × k` grid with XY routing, optionally with wraparound (a torus).
#[derive(Clone, Copy, Debug)]
pub struct Mesh2D {
    k: usize,
    wraparound: bool,
}

impl Mesh2D {
    /// The smallest square grid with room for `endpoints`.
    #[must_use]
    pub fn for_endpoints(endpoints: usize, wraparound: bool) -> Self {
        let mut k = 1;
        while k * k < endpoints.max(1) {
            k += 1;
        }
        Self { k: k.max(2), wraparound }
    }

    const fn coords(&self, node: usize) -> (usize, usize) {
        (node % self.k, node / self.k)
    }

    const fn node(&self, x: usize, y: usize) -> usize {
        y * self.k + x
    }

    /// Next coordinate towards `to` along one axis.
    const fn step(&self, from: usize, to: usize) -> usize {
        if from == to {
            return from;
        }
        if !self.wraparound {
            return if to > from { from + 1 } else { from - 1 };
        }
        let forward = (to + self.k - from) % self.k;
        if forward <= self.k - forward { (from + 1) % self.k } else { (from + self.k - 1) % self.k }
    }
}

impl NetworkTopology for Mesh2D {
    fn kind(&self) -> &'static str {
        if self.wraparound { "torus" } else { "mesh" }
    }

    fn node_count(&self) -> usize {
        self.k * self.k
    }

    fn neighbours(&self, node: usize) -> Vec<usize> {
        let (x, y) = self.coords(node);
        let mut out = Vec::with_capacity(4);
        if self.wraparound {
            out.push(self.node((x + self.k - 1) % self.k, y));
            out.push(self.node((x + 1) % self.k, y));
            out.push(self.node(x, (y + self.k - 1) % self.k));
            out.push(self.node(x, (y + 1) % self.k));
        } else {
            if x > 0 {
                out.push(self.node(x - 1, y));
            }
            if x + 1 < self.k {
                out.push(self.node(x + 1, y));
            }
            if y > 0 {
                out.push(self.node(x, y - 1));
            }
            if y + 1 < self.k {
                out.push(self.node(x, y + 1));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn next_hop(&self, from: usize, to: usize) -> usize {
        let (fx, fy) = self.coords(from);
        let (tx, ty) = self.coords(to);
        if fx == tx { self.node(fx, self.step(fy, ty)) } else { self.node(self.step(fx, tx), fy) }
    }

    fn diameter(&self) -> usize {
        if self.wraparound { 2 * (self.k / 2) } else { 2 * (self.k - 1) }
    }
}

/// A hypercube with dimension-order routing.
#[derive(Clone, Copy, Debug)]
pub struct Hypercube {
    dims: u32,
}

impl Hypercube {
    /// The smallest hypercube with room for `endpoints`.
    #[must_use]
    pub fn for_endpoints(endpoints: usize) -> Self {
        let mut dims = 1;
        while (1usize << dims) < endpoints.max(2) {
            dims += 1;
        }
        Self { dims }
    }
}

impl NetworkTopology for Hypercube {
    fn kind(&self) -> &'static str {
        "hypercube"
    }

    fn node_count(&self) -> usize {
        1 << self.dims
    }

    fn neighbours(&self, node: usize) -> Vec<usize> {
        (0..self.dims).map(|d| node ^ (1 << d)).collect()
    }

    fn next_hop(&self, from: usize, to: usize) -> usize {
        let differing = from ^ to;
        from ^ (1 << differing.trailing_zeros())
    }

    fn diameter(&self) -> usize {
        self.dims as usize
    }
}

/// A message travelling a routed network.
#[derive(Clone, Copy, Debug)]
struct Routed {
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
    use crate::common::{LineAddr, PhysAddr};
    use crate::sim::components::ReqId;
    use crate::sim::packet::coherence::ReqKind;

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

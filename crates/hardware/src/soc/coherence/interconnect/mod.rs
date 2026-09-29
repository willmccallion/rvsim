//! Interconnects: the timing of moving a coherence message from one node
//! to another.
//!
//! An interconnect accepts messages from nodes, advances once per cycle,
//! and hands over the messages that arrive that cycle. Every
//! implementation keeps one queue per virtual channel ([`MsgClass`](crate::sim::packet::coherence::MsgClass)) so a
//! stalled request can never block a response behind it, and arbitrates
//! deterministically (oldest message first, then lowest port).

mod crossbar;
mod routed;
mod topology;

pub use crossbar::Crossbar;
pub use routed::RoutedNetwork;
pub use topology::{Hypercube, Mesh2D, NetworkTopology, Ring};

use crate::common::CoreId;
use crate::sim::packet::coherence::{CoherenceMsg, Node};
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

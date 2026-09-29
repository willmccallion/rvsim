//! Cache coherence for multi-core systems.
//!
//! Three separately swappable pieces, per `docs/architecture/multicore.md`:
//! a [`protocol::CoherenceProtocol`] (a pure state machine), a
//! [`home::HomeAgent`] (who must be snooped: broadcast or a precise snoop
//! filter), and an [`interconnect::Interconnect`] (the timing of moving a
//! message). [`fabric::CoherenceFabric`] composes them into the component
//! the private L2s talk to. [`audit`] checks the protocol's invariants over
//! the whole system.

pub mod audit;
pub mod fabric;
pub mod home;
pub mod interconnect;
pub mod protocol;
pub mod stats;

pub use fabric::{CoherenceFabric, FabricLayout};
pub use protocol::{CoherenceProtocol, CoreSet, Holders, Mesi};

use crate::config::{
    CoherenceConfig, CoherenceProtocolConfig, HomeAgentConfig, InterconnectConfig,
};
use crate::sim::components::ComponentId;
use home::{Broadcast, HomeAgent, SnoopFilter};
use interconnect::{Crossbar, Hypercube, Interconnect, Mesh2D, Ring, RoutedNetwork};
use stats::CoherenceStatPaths;

/// Sizes that shape the fabric, taken from the caches it sits between.
#[derive(Clone, Copy, Debug)]
pub struct FabricGeometry {
    /// Cache line size.
    pub line_bytes: usize,
    /// Lines held by all private L2s together.
    pub private_l2_lines: usize,
}

/// Builds the fabric `config` describes for `agents` (one L2 per core, in
/// core order) in front of `llc`.
#[must_use]
pub fn build(
    config: &CoherenceConfig,
    geometry: FabricGeometry,
    llc: ComponentId,
    agents: Vec<ComponentId>,
) -> CoherenceFabric {
    let cores = agents.len();
    let stat_paths = CoherenceStatPaths::new();
    let protocol: Box<dyn CoherenceProtocol> = match config.protocol {
        CoherenceProtocolConfig::Mesi => Box::new(Mesi),
    };
    let tracking: Box<dyn HomeAgent> = match config.home_agent {
        HomeAgentConfig::Broadcast => Box::new(Broadcast),
        HomeAgentConfig::SnoopFilter { capacity_factor, ways } => {
            let entries =
                (geometry.private_l2_lines as f64 * capacity_factor.max(0.0)).ceil() as usize;
            Box::new(SnoopFilter::new(entries.max(ways), ways, geometry.line_bytes as u64))
        }
    };
    let paths = stat_paths.interconnect;
    let interconnect: Box<dyn Interconnect> = match config.interconnect {
        InterconnectConfig::Crossbar { hop_latency, bytes_per_cycle } => {
            Box::new(Crossbar::new(cores, geometry.line_bytes, hop_latency, bytes_per_cycle, paths))
        }
        InterconnectConfig::Ring { hop_latency, bytes_per_cycle } => Box::new(RoutedNetwork::new(
            Ring::new(cores + 1),
            cores,
            geometry.line_bytes,
            hop_latency,
            bytes_per_cycle,
            paths,
        )),
        InterconnectConfig::Mesh { hop_latency, bytes_per_cycle } => Box::new(RoutedNetwork::new(
            Mesh2D::for_endpoints(cores + 1, false),
            cores,
            geometry.line_bytes,
            hop_latency,
            bytes_per_cycle,
            paths,
        )),
        InterconnectConfig::Torus { hop_latency, bytes_per_cycle } => Box::new(RoutedNetwork::new(
            Mesh2D::for_endpoints(cores + 1, true),
            cores,
            geometry.line_bytes,
            hop_latency,
            bytes_per_cycle,
            paths,
        )),
        InterconnectConfig::Hypercube { hop_latency, bytes_per_cycle } => {
            Box::new(RoutedNetwork::new(
                Hypercube::for_endpoints(cores + 1),
                cores,
                geometry.line_bytes,
                hop_latency,
                bytes_per_cycle,
                paths,
            ))
        }
    };
    let layout = FabricLayout {
        llc,
        agents,
        line_bytes: geometry.line_bytes,
        txn_capacity: config.txn_entries,
    };
    CoherenceFabric::new(protocol, tracking, interconnect, layout, stat_paths)
}

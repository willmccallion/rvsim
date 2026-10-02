//! The coherence fabric: protocol, home agent and interconnect.

use serde::Deserialize;

/// Which home agent decides who must be snooped.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "kind")]
pub enum HomeAgentConfig {
    /// Snoop every other core on every request.
    Broadcast,
    /// Exact sharer tracking in a set-associative filter.
    SnoopFilter {
        /// Tracked lines as a multiple of the aggregate private L2 lines.
        #[serde(default = "HomeAgentConfig::default_capacity_factor")]
        capacity_factor: f64,
        /// Filter associativity.
        #[serde(default = "HomeAgentConfig::default_ways")]
        ways: usize,
    },
}

impl HomeAgentConfig {
    const fn default_capacity_factor() -> f64 {
        1.5
    }

    const fn default_ways() -> usize {
        8
    }
}

impl Default for HomeAgentConfig {
    fn default() -> Self {
        Self::SnoopFilter {
            capacity_factor: Self::default_capacity_factor(),
            ways: Self::default_ways(),
        }
    }
}

/// Which interconnect carries coherence messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind")]
pub enum InterconnectConfig {
    /// Any port to any port, one hop.
    Crossbar {
        /// Cycles a message spends crossing.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes an output port moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// Bidirectional ring; the home is one stop.
    Ring {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// 2-D mesh with XY routing.
    Mesh {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// 2-D torus (mesh with wraparound) with XY routing.
    Torus {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
    /// Hypercube with dimension-order routing.
    Hypercube {
        /// Cycles per hop.
        #[serde(default = "InterconnectConfig::default_hop_latency")]
        hop_latency: u64,
        /// Bytes a link moves per cycle.
        #[serde(default = "InterconnectConfig::default_bytes_per_cycle")]
        bytes_per_cycle: usize,
    },
}

impl InterconnectConfig {
    const fn default_hop_latency() -> u64 {
        2
    }

    const fn default_bytes_per_cycle() -> usize {
        32
    }
}

impl Default for InterconnectConfig {
    fn default() -> Self {
        Self::Crossbar {
            hop_latency: Self::default_hop_latency(),
            bytes_per_cycle: Self::default_bytes_per_cycle(),
        }
    }
}

/// Coherence protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CoherenceProtocolConfig {
    /// Modified / Exclusive / Shared / Invalid.
    #[default]
    Mesi,
}

/// Coherence fabric configuration.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoherenceConfig {
    /// Protocol.
    #[serde(default)]
    pub protocol: CoherenceProtocolConfig,
    /// Home agent.
    #[serde(default)]
    pub home_agent: HomeAgentConfig,
    /// Interconnect.
    #[serde(default)]
    pub interconnect: InterconnectConfig,
    /// Transactions the home can have live at once.
    #[serde(default = "CoherenceConfig::default_txn_entries")]
    pub txn_entries: usize,
}

impl CoherenceConfig {
    const fn default_txn_entries() -> usize {
        32
    }
}

impl Default for CoherenceConfig {
    fn default() -> Self {
        Self {
            protocol: CoherenceProtocolConfig::default(),
            home_agent: HomeAgentConfig::default(),
            interconnect: InterconnectConfig::default(),
            txn_entries: Self::default_txn_entries(),
        }
    }
}

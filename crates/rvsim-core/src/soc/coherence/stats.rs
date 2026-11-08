//! Stat paths for the coherence fabric, allocated once when it is built.

use crate::sim::stats::{StatSource, Stats};

crate::sim::stats::paths::stat_paths! {
    /// Counters the home agent writes, under `coherence.ha`.
    HomeStatPaths {
        /// `ReadShared` requests received.
        read_shared: "requests.read_shared" => Events,
        /// `ReadUnique` requests received.
        read_unique: "requests.read_unique" => Events,
        /// `CleanUnique` requests received.
        clean_unique: "requests.clean_unique" => Events,
        /// Writebacks received.
        writebacks: "requests.writebacks" => Events,
        /// Silent evictions received.
        evicts: "requests.evicts" => Events,
        /// Cache-maintenance requests received.
        maintenance: "requests.maintenance" => Events,
        /// Writebacks from a core a snoop had already taken the line from.
        stale_writebacks: "requests.stale_writebacks" => Events,
        /// Accesses carried to memory without snooping.
        non_coherent: "requests.non_coherent" => Events,
        /// Snoops sent.
        snoops_sent: "snoops_sent" => Events,
        /// Requests answered with another core's modified data.
        c2c_transfers: "c2c_transfers" => Events,
        /// Lines recalled because the tracking structure ran out of room.
        recalls: "recalls" => Events,
        /// Requests that waited for an earlier transaction on their line.
        serialised: "serialised" => Events,
        /// Requests that waited for a free transaction entry.
        txn_full_stalls: "txn_full_stalls" => Events,
        /// Tracking lookups that found the line.
        filter_hits: "filter.hits" => Events,
        /// Tracking lookups that found nothing.
        filter_misses: "filter.misses" => Events,
    }
    histograms {
        /// Request-to-completion latency in cycles (histogram).
        txn_latency: "txn_latency" => Cycles,
    }
}

crate::sim::stats::paths::stat_paths! {
    /// Counters an interconnect writes, under `coherence.interconnect`.
    InterconnectStatPaths {
        /// Messages transferred.
        messages: "messages" => Events,
        /// Bytes transferred.
        bytes: "bytes" => Events,
        /// Cycles a message waited for a busy link or port.
        blocked_cycles: "blocked_cycles" => Cycles,
        /// Port-class-cycles an output was busy transferring.
        busy_cycles: "busy_cycles" => Cycles,
    }
}

/// All fabric paths.
#[derive(Clone, Copy, Debug)]
pub struct CoherenceStatPaths {
    /// Home agent counters.
    pub home: HomeStatPaths,
    /// Interconnect counters.
    pub interconnect: InterconnectStatPaths,
}

impl CoherenceStatPaths {
    /// Paths under `coherence`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            home: HomeStatPaths::under("coherence.ha"),
            interconnect: InterconnectStatPaths::under("coherence.interconnect"),
        }
    }
}

impl Default for CoherenceStatPaths {
    fn default() -> Self {
        Self::new()
    }
}

impl StatSource for CoherenceStatPaths {
    fn register(&self, s: &mut Stats) {
        self.home.register_counters(s);
        self.interconnect.register_counters(s);
    }
}

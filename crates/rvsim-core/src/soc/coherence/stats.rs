//! Stat paths for the coherence fabric, allocated once when it is built.

use crate::sim::stats::{Meta, StatId, StatSource, Stats};

/// Counters the home agent writes, under `coherence.ha`.
#[derive(Clone, Copy, Debug)]
pub struct HomeStatPaths {
    /// `ReadShared` requests received.
    pub read_shared: StatId,
    /// `ReadUnique` requests received.
    pub read_unique: StatId,
    /// `CleanUnique` requests received.
    pub clean_unique: StatId,
    /// Writebacks received.
    pub writebacks: StatId,
    /// Silent evictions received.
    pub evicts: StatId,
    /// Cache-maintenance requests received.
    pub maintenance: StatId,
    /// Writebacks from a core a snoop had already taken the line from.
    pub stale_writebacks: StatId,
    /// Accesses carried to memory without snooping.
    pub non_coherent: StatId,
    /// Snoops sent.
    pub snoops_sent: StatId,
    /// Requests answered with another core's modified data.
    pub c2c_transfers: StatId,
    /// Lines recalled because the tracking structure ran out of room.
    pub recalls: StatId,
    /// Requests that waited for an earlier transaction on their line.
    pub serialised: StatId,
    /// Requests that waited for a free transaction entry.
    pub txn_full_stalls: StatId,
    /// Request-to-completion latency in cycles (histogram).
    pub txn_latency: StatId,
    /// Tracking lookups that found the line.
    pub filter_hits: StatId,
    /// Tracking lookups that found nothing.
    pub filter_misses: StatId,
}

impl HomeStatPaths {
    /// Paths under `subject`.
    #[must_use]
    pub fn new(subject: &str) -> Self {
        let path = |tail: &str| StatId::of(&format!("{subject}.{tail}"));
        Self {
            read_shared: path("requests.read_shared"),
            read_unique: path("requests.read_unique"),
            clean_unique: path("requests.clean_unique"),
            writebacks: path("requests.writebacks"),
            evicts: path("requests.evicts"),
            maintenance: path("requests.maintenance"),
            stale_writebacks: path("requests.stale_writebacks"),
            non_coherent: path("requests.non_coherent"),
            snoops_sent: path("snoops_sent"),
            c2c_transfers: path("c2c_transfers"),
            recalls: path("recalls"),
            serialised: path("serialised"),
            txn_full_stalls: path("txn_full_stalls"),
            txn_latency: path("txn_latency"),
            filter_hits: path("filter.hits"),
            filter_misses: path("filter.misses"),
        }
    }
}

/// Counters an interconnect writes, under `coherence.interconnect`.
#[derive(Clone, Copy, Debug)]
pub struct InterconnectStatPaths {
    /// Messages transferred.
    pub messages: StatId,
    /// Bytes transferred.
    pub bytes: StatId,
    /// Cycles a message waited for a busy link or port.
    pub blocked_cycles: StatId,
    /// Port-class-cycles an output was busy transferring.
    pub busy_cycles: StatId,
}

impl InterconnectStatPaths {
    /// Paths under `subject`.
    #[must_use]
    pub fn new(subject: &str) -> Self {
        let path = |tail: &str| StatId::of(&format!("{subject}.{tail}"));
        Self {
            messages: path("messages"),
            bytes: path("bytes"),
            blocked_cycles: path("blocked_cycles"),
            busy_cycles: path("busy_cycles"),
        }
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
            home: HomeStatPaths::new("coherence.ha"),
            interconnect: InterconnectStatPaths::new("coherence.interconnect"),
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
        let h = &self.home;
        s.register(h.read_shared, Meta::events("ReadShared requests"));
        s.register(h.read_unique, Meta::events("ReadUnique requests"));
        s.register(h.clean_unique, Meta::events("CleanUnique (upgrade) requests"));
        s.register(h.writebacks, Meta::events("writebacks from private caches"));
        s.register(h.evicts, Meta::events("silent evictions reported by private caches"));
        s.register(h.maintenance, Meta::events("cache-maintenance requests from private caches"));
        s.register(
            h.stale_writebacks,
            Meta::events("writebacks whose line a snoop had already collected"),
        );
        s.register(h.non_coherent, Meta::events("accesses carried to memory without snooping"));
        s.register(h.snoops_sent, Meta::events("snoops sent"));
        s.register(
            h.c2c_transfers,
            Meta::events("requests served from another core's modified copy"),
        );
        s.register(h.recalls, Meta::events("lines recalled to free tracking room"));
        s.register(
            h.serialised,
            Meta::events("requests that waited for an earlier transaction on their line"),
        );
        s.register(h.txn_full_stalls, Meta::events("requests that waited for a transaction entry"));
        s.register(h.filter_hits, Meta::events("tracking lookups that found the line"));
        s.register(h.filter_misses, Meta::events("tracking lookups that found nothing"));
        s.register_histogram(
            h.txn_latency,
            Meta::cycles("cycles from a request's arrival at the home to its completion"),
        );
        let i = &self.interconnect;
        s.register(i.messages, Meta::events("messages transferred"));
        s.register(i.bytes, Meta::events("bytes transferred"));
        s.register(
            i.blocked_cycles,
            Meta::cycles("message-cycles spent waiting for a busy link or port"),
        );
        s.register(i.busy_cycles, Meta::cycles("port-class-cycles spent transferring"));
    }
}

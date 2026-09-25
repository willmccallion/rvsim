//! Stat paths for the coherence fabric, allocated once when it is built.

fn leak(path: String) -> &'static str {
    Box::leak(path.into_boxed_str())
}

/// Counters the home agent writes, under `coherence.ha`.
#[derive(Clone, Copy, Debug)]
pub struct HomeStatPaths {
    /// `ReadShared` requests received.
    pub read_shared: &'static str,
    /// `ReadUnique` requests received.
    pub read_unique: &'static str,
    /// `CleanUnique` requests received.
    pub clean_unique: &'static str,
    /// Writebacks received.
    pub writebacks: &'static str,
    /// Silent evictions received.
    pub evicts: &'static str,
    /// Writebacks from a core a snoop had already taken the line from.
    pub stale_writebacks: &'static str,
    /// Accesses carried to memory without snooping.
    pub non_coherent: &'static str,
    /// Snoops sent.
    pub snoops_sent: &'static str,
    /// Requests answered with another core's modified data.
    pub c2c_transfers: &'static str,
    /// Lines recalled because the tracking structure ran out of room.
    pub recalls: &'static str,
    /// Requests that waited for an earlier transaction on their line.
    pub serialised: &'static str,
    /// Requests that waited for a free transaction entry.
    pub txn_full_stalls: &'static str,
    /// Request-to-completion latency in cycles (histogram).
    pub txn_latency: &'static str,
    /// Tracking lookups that found the line.
    pub filter_hits: &'static str,
    /// Tracking lookups that found nothing.
    pub filter_misses: &'static str,
}

impl HomeStatPaths {
    /// Paths under `subject`.
    #[must_use]
    pub fn new(subject: &str) -> Self {
        let path = |tail: &str| leak(format!("{subject}.{tail}"));
        Self {
            read_shared: path("requests.read_shared"),
            read_unique: path("requests.read_unique"),
            clean_unique: path("requests.clean_unique"),
            writebacks: path("requests.writebacks"),
            evicts: path("requests.evicts"),
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
    pub messages: &'static str,
    /// Bytes transferred.
    pub bytes: &'static str,
    /// Cycles a message waited for a busy link or port.
    pub blocked_cycles: &'static str,
    /// Port-class-cycles an output was busy transferring.
    pub busy_cycles: &'static str,
}

impl InterconnectStatPaths {
    /// Paths under `subject`.
    #[must_use]
    pub fn new(subject: &str) -> Self {
        let path = |tail: &str| leak(format!("{subject}.{tail}"));
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
        Self { home: HomeStatPaths::new("coherence.ha"), interconnect: InterconnectStatPaths::new("coherence.interconnect") }
    }
}

impl Default for CoherenceStatPaths {
    fn default() -> Self {
        Self::new()
    }
}

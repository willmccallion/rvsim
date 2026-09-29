//! Stat paths for one cache, allocated when the cache is built.

use crate::sim::stats::StatId;

/// Counters one cache writes, rooted at its subject (`core0.cache.l1d`,
/// `llc`, ...).
#[derive(Clone, Copy, Debug)]
pub struct CacheStatPaths {
    /// Requests answered from the tag array.
    pub hits: StatId,
    /// Requests that started or joined a line fetch.
    pub misses: StatId,
    /// Misses that joined an MSHR already fetching the line.
    pub mshr_hits: StatId,
    /// Requests queued because every MSHR or writeback buffer entry was busy.
    pub blocked_requests: StatId,
    /// Lines installed by a fill.
    pub fills: StatId,
    /// Valid lines replaced by a fill.
    pub evictions: StatId,
    /// Lines written back to the next level.
    pub writebacks: StatId,
    /// Lines invalidated on request of the next level.
    pub back_invalidations: StatId,
    /// Cache-maintenance operations (`cbo.clean` / `flush` / `inval`) passed
    /// through this cache.
    pub maintenance: StatId,
    /// Probes received on behalf of a snoop.
    pub probes: StatId,
    /// Snoops received from the home agent (coherent L2 only).
    pub snoops: StatId,
    /// Snoops that took the line away.
    pub snoop_invalidations: StatId,
    /// Snoops that left a shared copy.
    pub snoop_downgrades: StatId,
    /// Permission requests for a line held Shared (`CleanUnique`).
    pub upgrades: StatId,
    /// Permission grants that arrived after a snoop took the line, re-issued
    /// as `ReadUnique`.
    pub upgrade_retries: StatId,
    /// Prefetch fetches started.
    pub prefetches_issued: StatId,
    /// Prefetch fetches a demand request joined before the fill arrived.
    pub prefetches_useful: StatId,
    /// Derived: misses / (hits + misses).
    pub miss_rate: StatId,
}

impl CacheStatPaths {
    /// Paths under `subject`.
    #[must_use]
    pub fn new(subject: &str) -> Self {
        let path = |tail: &str| StatId::of(&format!("{subject}.{tail}"));
        Self {
            hits: path("hits"),
            misses: path("misses"),
            mshr_hits: path("mshr_hits"),
            blocked_requests: path("blocked_requests"),
            fills: path("fills"),
            evictions: path("evictions"),
            writebacks: path("writebacks"),
            back_invalidations: path("back_invalidations"),
            maintenance: path("maintenance"),
            probes: path("probes"),
            snoops: path("coherence.snoops"),
            snoop_invalidations: path("coherence.invalidations"),
            snoop_downgrades: path("coherence.downgrades"),
            upgrades: path("coherence.upgrades"),
            upgrade_retries: path("coherence.upgrade_retries"),
            prefetches_issued: path("prefetches.issued"),
            prefetches_useful: path("prefetches.useful"),
            miss_rate: path("miss_rate"),
        }
    }
}

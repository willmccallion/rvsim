//! Stat paths for one cache, allocated when the cache is built.

fn leak(path: String) -> &'static str {
    Box::leak(path.into_boxed_str())
}

/// Counters one cache writes, rooted at its subject (`core0.cache.l1d`,
/// `llc`, ...).
#[derive(Clone, Copy, Debug)]
pub struct CacheStatPaths {
    /// Requests answered from the tag array.
    pub hits: &'static str,
    /// Requests that started or joined a line fetch.
    pub misses: &'static str,
    /// Misses that joined an MSHR already fetching the line.
    pub mshr_hits: &'static str,
    /// Requests queued because every MSHR or writeback buffer entry was busy.
    pub blocked_requests: &'static str,
    /// Lines installed by a fill.
    pub fills: &'static str,
    /// Valid lines replaced by a fill.
    pub evictions: &'static str,
    /// Lines written back to the next level.
    pub writebacks: &'static str,
    /// Lines invalidated on request of the next level.
    pub back_invalidations: &'static str,
    /// Probes received on behalf of a snoop.
    pub probes: &'static str,
    /// Snoops received from the home agent (coherent L2 only).
    pub snoops: &'static str,
    /// Snoops that took the line away.
    pub snoop_invalidations: &'static str,
    /// Snoops that left a shared copy.
    pub snoop_downgrades: &'static str,
    /// Permission requests for a line held Shared (`CleanUnique`).
    pub upgrades: &'static str,
    /// Permission grants that arrived after a snoop took the line, re-issued
    /// as `ReadUnique`.
    pub upgrade_retries: &'static str,
    /// Prefetch fetches started.
    pub prefetches_issued: &'static str,
    /// Prefetch fetches a demand request joined before the fill arrived.
    pub prefetches_useful: &'static str,
    /// Derived: misses / (hits + misses).
    pub miss_rate: &'static str,
}

impl CacheStatPaths {
    /// Paths under `subject`.
    #[must_use]
    pub fn new(subject: &str) -> Self {
        let path = |tail: &str| leak(format!("{subject}.{tail}"));
        Self {
            hits: path("hits"),
            misses: path("misses"),
            mshr_hits: path("mshr_hits"),
            blocked_requests: path("blocked_requests"),
            fills: path("fills"),
            evictions: path("evictions"),
            writebacks: path("writebacks"),
            back_invalidations: path("back_invalidations"),
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

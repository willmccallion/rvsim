//! Stat paths for one cache, allocated when the cache is built.

use crate::sim::stats::{Formula, Meta, StatId, StatSource, Stats};

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
    /// Prefetch candidates dropped for lying outside the 4 KiB page of the
    /// access that produced them.
    pub prefetches_page_crossing: StatId,
    /// Prefetch requests from above dropped for want of a free MSHR.
    pub prefetches_dropped: StatId,
    /// Prefetches the store-miss prefetcher sent to the next level.
    pub store_prefetches: StatId,
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
            prefetches_page_crossing: path("prefetches.page_crossing"),
            prefetches_dropped: path("prefetches.dropped"),
            store_prefetches: path("prefetches.store_stream"),
            miss_rate: path("miss_rate"),
        }
    }
}

impl StatSource for CacheStatPaths {
    fn register(&self, s: &mut Stats) {
        s.register(self.hits, Meta::events("requests answered from the tag array"));
        s.register(self.misses, Meta::events("requests that started or joined a line fetch"));
        s.register(self.mshr_hits, Meta::events("misses that joined an in-flight fetch"));
        s.register(
            self.blocked_requests,
            Meta::events("requests queued while MSHRs or writeback buffer were full"),
        );
        s.register(self.fills, Meta::events("lines installed"));
        s.register(self.evictions, Meta::events("valid lines replaced"));
        s.register(self.writebacks, Meta::events("lines written to the next level"));
        s.register(
            self.back_invalidations,
            Meta::events("lines dropped at the next level's request"),
        );
        s.register(self.maintenance, Meta::events("cache-maintenance operations passed through"));
        s.register(self.probes, Meta::events("probes received on behalf of snoops"));
        s.register(self.snoops, Meta::events("snoops received from the home agent"));
        s.register(self.snoop_invalidations, Meta::events("snoops that took the line away"));
        s.register(self.snoop_downgrades, Meta::events("snoops that left a shared copy"));
        s.register(self.upgrades, Meta::events("permission requests for lines held Shared"));
        s.register(
            self.upgrade_retries,
            Meta::events("permission grants that arrived after a snoop took the line"),
        );
        s.register(self.prefetches_issued, Meta::events("prefetch fetches started"));
        s.register(
            self.prefetches_useful,
            Meta::events("prefetch fetches a demand request joined"),
        );
        s.register(
            self.prefetches_page_crossing,
            Meta::events("prefetch candidates dropped at a 4 KiB page boundary"),
        );
        s.register(
            self.prefetches_dropped,
            Meta::events("prefetch requests dropped for want of an MSHR"),
        );
        s.register(
            self.store_prefetches,
            Meta::events("store-miss prefetches sent to the next level"),
        );
        s.derive(
            self.miss_rate,
            Formula::Ratio { numerator: self.misses, other: self.hits },
            Meta::ratio("miss rate"),
        );
    }
}

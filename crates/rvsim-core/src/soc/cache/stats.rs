//! Stat paths for one cache, allocated when the cache is built.

use crate::sim::stats::{Formula, StatId, StatSource, Stats, Unit};

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
    /// Probes received from the level below, for a snoop or an
    /// instruction fetch.
    pub probes: StatId,
    /// Instruction fetches that probed a data cache above for a writable
    /// copy of their line before being served.
    pub fetch_probes: StatId,
    /// Fetch probes that found the line dirty.
    pub fetch_probes_dirty: StatId,
    /// Whole-cache flushes (FENCE.I with no cache below keeping instruction
    /// fetches coherent).
    pub flushes: StatId,
    /// Valid lines those flushes invalidated.
    pub flushed_lines: StatId,
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
    /// Prefetch fetches a request from above joined before the fill
    /// arrived: the prefetch was late.
    pub prefetches_late: StatId,
    /// Prefetched lines a request from above found after they were
    /// installed.
    pub prefetches_useful: StatId,
    /// Prefetched lines dropped before any request from above found them.
    pub prefetches_unused: StatId,
    /// Derived: useful + late.
    pub prefetches_used: StatId,
    /// Derived: used / issued.
    pub prefetches_accuracy: StatId,
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
            fetch_probes: path("fetch_probes"),
            fetch_probes_dirty: path("fetch_probes_dirty"),
            flushes: path("flushes"),
            flushed_lines: path("flushed_lines"),
            snoops: path("coherence.snoops"),
            snoop_invalidations: path("coherence.invalidations"),
            snoop_downgrades: path("coherence.downgrades"),
            upgrades: path("coherence.upgrades"),
            upgrade_retries: path("coherence.upgrade_retries"),
            prefetches_issued: path("prefetches.issued"),
            prefetches_late: path("prefetches.late"),
            prefetches_useful: path("prefetches.useful"),
            prefetches_unused: path("prefetches.unused"),
            prefetches_used: path("prefetches.used"),
            prefetches_accuracy: path("prefetches.accuracy"),
            prefetches_page_crossing: path("prefetches.page_crossing"),
            prefetches_dropped: path("prefetches.dropped"),
            store_prefetches: path("prefetches.store_stream"),
            miss_rate: path("miss_rate"),
        }
    }
}

impl StatSource for CacheStatPaths {
    fn register(&self, s: &mut Stats) {
        s.register(self.hits, Unit::Events);
        s.register(self.misses, Unit::Events);
        s.register(self.mshr_hits, Unit::Events);
        s.register(self.blocked_requests, Unit::Events);
        s.register(self.fills, Unit::Events);
        s.register(self.evictions, Unit::Events);
        s.register(self.writebacks, Unit::Events);
        s.register(self.back_invalidations, Unit::Events);
        s.register(self.maintenance, Unit::Events);
        s.register(self.probes, Unit::Events);
        s.register(self.fetch_probes, Unit::Events);
        s.register(self.fetch_probes_dirty, Unit::Events);
        s.register(self.flushes, Unit::Events);
        s.register(self.flushed_lines, Unit::Events);
        s.register(self.snoops, Unit::Events);
        s.register(self.snoop_invalidations, Unit::Events);
        s.register(self.snoop_downgrades, Unit::Events);
        s.register(self.upgrades, Unit::Events);
        s.register(self.upgrade_retries, Unit::Events);
        s.register(self.prefetches_issued, Unit::Events);
        s.register(self.prefetches_late, Unit::Events);
        s.register(self.prefetches_useful, Unit::Events);
        s.register(self.prefetches_unused, Unit::Events);
        s.register(self.prefetches_page_crossing, Unit::Events);
        s.register(self.prefetches_dropped, Unit::Events);
        s.register(self.store_prefetches, Unit::Events);
        s.derive(
            self.prefetches_used,
            Formula::Sum(vec![self.prefetches_useful, self.prefetches_late]),
            Unit::Events,
        );
        s.derive(
            self.prefetches_accuracy,
            Formula::Div(self.prefetches_used, self.prefetches_issued),
            Unit::Ratio,
        );
        s.derive(
            self.miss_rate,
            Formula::Ratio { numerator: self.misses, other: self.hits },
            Unit::Ratio,
        );
    }
}

//! Stat paths for one cache, allocated when the cache is built.

use crate::sim::stats::{Formula, StatSource, Stats, Unit};

crate::sim::stats::paths::stat_paths! {
    /// Counters one cache writes, rooted at its subject (`core0.cache.l1d`,
    /// `llc`, ...).
    CacheStatPaths {
        /// Requests answered from the tag array.
        hits: "hits" => Events,
        /// Requests that started or joined a line fetch.
        misses: "misses" => Events,
        /// Misses that joined an MSHR already fetching the line.
        mshr_hits: "mshr_hits" => Events,
        /// Requests queued because every MSHR or writeback buffer entry was busy.
        blocked_requests: "blocked_requests" => Events,
        /// Lines installed by a fill.
        fills: "fills" => Events,
        /// Valid lines replaced by a fill.
        evictions: "evictions" => Events,
        /// Lines written back to the next level.
        writebacks: "writebacks" => Events,
        /// Lines invalidated on request of the next level.
        back_invalidations: "back_invalidations" => Events,
        /// Cache-maintenance operations (`cbo.clean` / `flush` / `inval`) passed
        /// through this cache.
        maintenance: "maintenance" => Events,
        /// Probes received from the level below, for a snoop or an
        /// instruction fetch.
        probes: "probes" => Events,
        /// Instruction fetches that probed a data cache above for a writable
        /// copy of their line before being served.
        fetch_probes: "fetch_probes" => Events,
        /// Fetch probes that found the line dirty.
        fetch_probes_dirty: "fetch_probes_dirty" => Events,
        /// Whole-cache flushes (FENCE.I with no cache below keeping instruction
        /// fetches coherent).
        flushes: "flushes" => Events,
        /// Valid lines those flushes invalidated.
        flushed_lines: "flushed_lines" => Events,
        /// Snoops received from the home agent (coherent L2 only).
        snoops: "coherence.snoops" => Events,
        /// Snoops that took the line away.
        snoop_invalidations: "coherence.invalidations" => Events,
        /// Snoops that left a shared copy.
        snoop_downgrades: "coherence.downgrades" => Events,
        /// Permission requests for a line held Shared (`CleanUnique`).
        upgrades: "coherence.upgrades" => Events,
        /// Permission grants that arrived after a snoop took the line, re-issued
        /// as `ReadUnique`.
        upgrade_retries: "coherence.upgrade_retries" => Events,
        /// Prefetch fetches started.
        prefetches_issued: "prefetches.issued" => Events,
        /// Prefetch fetches a request from above joined before the fill
        /// arrived: the prefetch was late.
        prefetches_late: "prefetches.late" => Events,
        /// Prefetched lines a request from above found after they were
        /// installed.
        prefetches_useful: "prefetches.useful" => Events,
        /// Prefetched lines dropped before any request from above found them.
        prefetches_unused: "prefetches.unused" => Events,
        /// Prefetch candidates dropped for lying outside the 4 KiB page of the
        /// access that produced them.
        prefetches_page_crossing: "prefetches.page_crossing" => Events,
        /// Prefetch requests from above dropped for want of a free MSHR.
        prefetches_dropped: "prefetches.dropped" => Events,
        /// Prefetches the store-miss prefetcher sent to the next level.
        store_prefetches: "prefetches.store_stream" => Events,
    }
    derived {
        /// Derived: useful + late.
        prefetches_used: "prefetches.used",
        /// Derived: used / issued.
        prefetches_accuracy: "prefetches.accuracy",
        /// Derived: misses / (hits + misses).
        miss_rate: "miss_rate",
    }
}

impl StatSource for CacheStatPaths {
    fn register(&self, s: &mut Stats) {
        self.register_counters(s);
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

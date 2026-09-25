//! Component identifiers for the whole system, derived once from config.
//!
//! `Topology` is the only place that assigns `CoreId`, `HartId`,
//! `PipelineId`, `CacheId` and `MemCtrlId` values. Dispatch code asks it
//! where an ID lives instead of relying on numbering conventions, so
//! adding a cache level, a slice or a memory channel is a change here, not
//! in every router.

use crate::common::{CoreId, HartId};
use crate::sim::components::{CacheId, MemCtrlId, PipelineId};

/// Private caches a core owns, in the order their IDs are allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateCache {
    /// L1 instruction cache.
    L1I,
    /// L1 data cache.
    L1D,
    /// Private unified L2.
    L2,
}

/// Number of private caches per core.
const PRIVATE_CACHES_PER_CORE: u32 = 3;

/// The IDs belonging to one core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTopology {
    /// The core.
    pub core_id: CoreId,
    /// Harts resident on the core (one unless SMT).
    pub hart_ids: Vec<HartId>,
    /// The core's pipeline.
    pub pipeline_id: PipelineId,
    /// L1 instruction cache.
    pub l1i: CacheId,
    /// L1 data cache.
    pub l1d: CacheId,
    /// Private L2.
    pub l2: CacheId,
}

impl CoreTopology {
    /// ID of one of the core's private caches.
    #[must_use]
    pub const fn cache(&self, which: PrivateCache) -> CacheId {
        match which {
            PrivateCache::L1I => self.l1i,
            PrivateCache::L1D => self.l1d,
            PrivateCache::L2 => self.l2,
        }
    }
}

/// Where a `CacheId` lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheSlot {
    /// One of a core's private caches.
    Private {
        /// Owning core.
        core: CoreId,
        /// Which cache.
        which: PrivateCache,
    },
    /// The shared last-level cache.
    Llc,
}

/// Every component ID in the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topology {
    /// Cores in `CoreId` order.
    pub cores: Vec<CoreTopology>,
    /// The shared last-level cache.
    pub llc: CacheId,
    /// Memory controllers in `MemCtrlId` order.
    pub mem_ctrls: Vec<MemCtrlId>,
}

impl Topology {
    /// One hart per core, `core_count` cores, one memory controller.
    ///
    /// # Panics
    ///
    /// Panics if `core_count` is zero.
    #[must_use]
    pub fn single_threaded_cores(core_count: usize) -> Self {
        assert!(core_count > 0, "a system needs at least one core");
        let cores = (0..core_count)
            .map(|c| {
                let c32 = u32::try_from(c).unwrap_or(u32::MAX);
                let base = c32 * PRIVATE_CACHES_PER_CORE;
                CoreTopology {
                    core_id: CoreId::new(c32),
                    hart_ids: vec![HartId::new(c32)],
                    pipeline_id: PipelineId::new(c32),
                    l1i: CacheId::new(base),
                    l1d: CacheId::new(base + 1),
                    l2: CacheId::new(base + 2),
                }
            })
            .collect();
        let llc = CacheId::new(u32::try_from(core_count).unwrap_or(u32::MAX) * PRIVATE_CACHES_PER_CORE);
        Self { cores, llc, mem_ctrls: vec![MemCtrlId::new(0)] }
    }

    /// Number of cores.
    #[must_use]
    pub const fn core_count(&self) -> usize {
        self.cores.len()
    }

    /// Number of harts across all cores.
    #[must_use]
    pub fn hart_count(&self) -> usize {
        self.cores.iter().map(|c| c.hart_ids.len()).sum()
    }

    /// The core hosting `hart`.
    #[must_use]
    pub fn core_of_hart(&self, hart: HartId) -> Option<CoreId> {
        self.cores.iter().find(|c| c.hart_ids.contains(&hart)).map(|c| c.core_id)
    }

    /// Where `cache` lives, or `None` for an unknown ID.
    #[must_use]
    pub fn locate_cache(&self, cache: CacheId) -> Option<CacheSlot> {
        if cache == self.llc {
            return Some(CacheSlot::Llc);
        }
        let index = cache.val() / PRIVATE_CACHES_PER_CORE;
        let core = self.cores.get(index as usize)?;
        let which = match cache.val() % PRIVATE_CACHES_PER_CORE {
            0 => PrivateCache::L1I,
            1 => PrivateCache::L1D,
            _ => PrivateCache::L2,
        };
        Some(CacheSlot::Private { core: core.core_id, which })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dense_and_the_llc_follows_the_last_core() {
        let t = Topology::single_threaded_cores(2);
        assert_eq!(t.cores[1].l1i, CacheId::new(3));
        assert_eq!(t.cores[1].l2, CacheId::new(5));
        assert_eq!(t.llc, CacheId::new(6));
        assert_eq!(t.locate_cache(CacheId::new(4)), Some(CacheSlot::Private { core: CoreId::new(1), which: PrivateCache::L1D }));
        assert_eq!(t.locate_cache(CacheId::new(6)), Some(CacheSlot::Llc));
        assert_eq!(t.locate_cache(CacheId::new(7)), None);
        assert_eq!(t.core_of_hart(HartId::new(1)), Some(CoreId::new(1)));
        assert_eq!(t.hart_count(), 2);
    }
}

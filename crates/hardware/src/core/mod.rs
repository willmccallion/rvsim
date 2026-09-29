//! Core processor implementation.
//!
//! This module contains the main CPU implementation including the instruction
//! pipeline, execution units, architecture-specific components, and the
//! orchestrator that coordinates all components.

/// Instruction pipeline implementation (stages and latches).
pub mod pipeline;

/// Execution units (ALU, FPU, LSU, MMU, branch predictor, cache, prefetcher).
pub mod units;

/// Vector unit timing: lane occupancy and result chaining.
pub mod vector;

use crate::common::CoreId;
use crate::config::{Config, InclusionPolicy};
use crate::core::pipeline::engine::PipelineDispatch;
use crate::core::pipeline::write_buffer::WriteCombiningBuffer;
use crate::core::units::bru::BranchPredictorWrapper;
use crate::core::units::cache::Cache;
use crate::core::units::mmu::Mmu;
use crate::core::units::mmu::tlb::TlbGeometry;
use crate::sim::components::{CacheId, ComponentId};
use crate::sim::packet::CacheLevel;
use crate::sim::stats::paths::CorePaths;

/// One processor core: the pipeline and the functional units it drives.
///
/// The pipeline is kept apart from the units so it can run with the units,
/// its hart and the uncore borrowed through a
/// [`CoreCtx`](crate::sim::CoreCtx).
#[derive(Debug)]
pub struct Core {
    /// The core's functional units.
    pub units: CoreUnits,
    /// The pipeline driving them.
    pub pipeline: PipelineDispatch,
}

/// A core's private hardware: its caches (L1 instruction, L1 data, L2),
/// MMU, write-combining buffer and branch predictor.
///
/// Configuration-derived constants (ELEN/Zvfh, inclusion policy, …) are not
/// cached here; they are read from the config on demand.
#[derive(Debug)]
pub struct CoreUnits {
    /// Identifier for this physical core within the `SoC`.
    pub core_id: CoreId,
    /// L1 Instruction Cache.
    pub l1_i_cache: Cache,
    /// L1 Data Cache.
    pub l1_d_cache: Cache,
    /// L2 Unified Cache.
    pub l2_cache: Cache,
    /// Address translation: the TLBs and the page-table walker.
    pub mmu: Mmu,
    /// Write Combining Buffer for store coalescing.
    pub wcb: WriteCombiningBuffer,
    /// Branch Predictor Unit.
    pub branch_predictor: BranchPredictorWrapper,
    /// Stat paths rooted at `core<N>`.
    pub stat_paths: CorePaths,
}

impl CoreUnits {
    /// Creates a new `Core` from configuration.
    ///
    /// Caches are assigned `CacheId`s starting at `cache_id_base`. The L1I/L1D
    /// caches are wired to forward downstream to the L2; the L2 is wired to
    /// forward downstream to the shared LLC (`l3_id`). The shared LLC adds
    /// this L2 as an upstream consumer separately.
    pub fn new(core_id: CoreId, config: &Config, cache_id_base: u32, l3_id: CacheId) -> Self {
        let l1i_id = CacheId::new(cache_id_base);
        let l1d_id = CacheId::new(cache_id_base + 1);
        let l2_id = CacheId::new(cache_id_base + 2);
        let subject = format!("core{}.cache", core_id.val());
        let inclusion = config.cache.inclusion_policy;

        let mut l1_i_cache =
            Cache::new(l1i_id, CacheLevel::L1I, &config.cache.l1_i, &format!("{subject}.l1i"));
        l1_i_cache.set_downstream(ComponentId::Cache(l2_id));

        let mut l1_d_cache =
            Cache::new(l1d_id, CacheLevel::L1D, &config.cache.l1_d, &format!("{subject}.l1d"));
        l1_d_cache.set_downstream(ComponentId::Cache(l2_id));
        l1_d_cache.set_clean_victims_to_downstream(inclusion == InclusionPolicy::Exclusive);

        let mut l2_cache =
            Cache::new(l2_id, CacheLevel::L2, &config.cache.l2, &format!("{subject}.l2"));
        l2_cache.set_downstream(ComponentId::Cache(l3_id));
        l2_cache.add_upstream(ComponentId::Cache(l1i_id));
        l2_cache.add_upstream(ComponentId::Cache(l1d_id));
        l2_cache.set_upstream_inclusion(inclusion);

        Self {
            core_id,
            l1_i_cache,
            l1_d_cache,
            l2_cache,
            mmu: Mmu::new(
                TlbGeometry { entries: config.memory.tlb_size, ways: config.memory.tlb_ways },
                TlbGeometry { entries: config.memory.l2_tlb_size, ways: config.memory.l2_tlb_ways },
                config.memory.l2_tlb_latency,
                config.memory.paging_mode_max,
            ),
            wcb: WriteCombiningBuffer::new(config.cache.wcb_entries, config.cache.l1_d.line_bytes),
            branch_predictor: BranchPredictorWrapper::new(config),
            stat_paths: CorePaths::new(core_id),
        }
    }
}

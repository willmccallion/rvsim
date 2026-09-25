//! Core processor implementation.
//!
//! This module contains the main CPU implementation including the instruction
//! pipeline, execution units, architecture-specific components, and the
//! orchestrator that coordinates all components.

/// Architecture-specific components (CSRs, register files, privilege modes, traps).
pub mod arch;

/// Per-thread RISC-V architectural state.
pub mod hart;

/// Instruction pipeline implementation (10-stage, latches, signals).
pub mod pipeline;

/// Execution units (ALU, FPU, LSU, MMU, branch predictor, cache, prefetcher).
pub mod units;

pub use self::hart::Hart;

use crate::common::CoreId;
use crate::config::Config;
use crate::core::pipeline::write_buffer::WriteCombiningBuffer;
use crate::core::units::bru::BranchPredictorWrapper;
use crate::core::units::cache::Cache;
use crate::core::units::cache::mshr::MshrFile;
use crate::core::units::prefetch::PrefetchFilter;
use crate::sim::components::{CacheId, ComponentId};
use crate::sim::packet::CacheLevel;
use crate::sim::stats::paths::CorePaths;

/// A physical processor core hosting one or more harts.
///
/// Owns the runtime hardware shared across the harts running on it:
/// pipeline-private caches (L1 instruction, L1 data, L2), load/store MSHRs,
/// write-combining buffer, branch predictor, and prefetch filter. Hosts one
/// or more [`Hart`]s — one for a non-SMT core, two or more for SMT.
///
/// Configuration-derived constants (`pipeline_width`, ELEN/Zvfh, inclusion
/// policy, …) are NOT cached here — they're read on demand from
/// `soc.config`. Pipeline properties like register-renaming are queried via
/// the [`ExecutionEngine`](crate::core::pipeline::engine::ExecutionEngine)
/// trait. `i_cache_line_bytes` is read via [`Cache::line_bytes()`].
#[derive(Debug)]
pub struct Core {
    /// Identifier for this physical core within the `SoC`.
    pub core_id: CoreId,
    /// L1 Instruction Cache.
    pub l1_i_cache: Cache,
    /// L1 Data Cache.
    pub l1_d_cache: Cache,
    /// L2 Unified Cache.
    pub l2_cache: Cache,
    /// L1D MSHR file for non-blocking cache access (O3 backend only).
    pub l1d_mshrs: MshrFile,
    /// Write Combining Buffer for store coalescing.
    pub wcb: WriteCombiningBuffer,
    /// Shared prefetch filter to deduplicate prefetch requests across cache levels.
    pub prefetch_filter: PrefetchFilter,
    /// Branch Predictor Unit.
    pub branch_predictor: BranchPredictorWrapper,
    /// Stat paths rooted at `core<N>`.
    pub stat_paths: CorePaths,
}

impl Core {
    /// Creates a new `Core` from configuration.
    ///
    /// Caches are assigned `CacheId`s starting at `cache_id_base`. The L1I/L1D
    /// caches are wired to forward downstream to the L2; the L2 is wired to
    /// forward downstream to the shared LLC (`l3_id`). The shared LLC adds
    /// this L2 as an upstream consumer separately.
    pub fn new(core_id: CoreId, config: &Config, cache_id_base: u32, l3_id: CacheId) -> Self {
        let prefetch_window = if config.cache.l1_d.prefetcher != crate::config::Prefetcher::None
            || config.cache.l2.prefetcher != crate::config::Prefetcher::None
        {
            64
        } else {
            0
        };

        let l1i_id = CacheId::new(cache_id_base);
        let l1d_id = CacheId::new(cache_id_base + 1);
        let l2_id = CacheId::new(cache_id_base + 2);

        let mut l1_i_cache = Cache::new(l1i_id, CacheLevel::L1I, &config.cache.l1_i);
        l1_i_cache.set_downstream(ComponentId::Cache(l2_id));

        let mut l1_d_cache = Cache::new(l1d_id, CacheLevel::L1D, &config.cache.l1_d);
        l1_d_cache.set_downstream(ComponentId::Cache(l2_id));

        let mut l2_cache = Cache::new(l2_id, CacheLevel::L2, &config.cache.l2);
        l2_cache.set_downstream(ComponentId::Cache(l3_id));
        l2_cache.add_upstream(ComponentId::Cache(l1i_id));
        l2_cache.add_upstream(ComponentId::Cache(l1d_id));

        Self {
            core_id,
            l1_i_cache,
            l1_d_cache,
            l2_cache,
            l1d_mshrs: MshrFile::new(config.cache.l1_d.mshr_count, config.cache.l1_d.line_bytes),
            wcb: WriteCombiningBuffer::new(config.cache.wcb_entries, config.cache.l1_d.line_bytes),
            prefetch_filter: PrefetchFilter::new(prefetch_window, config.cache.l1_d.line_bytes),
            branch_predictor: BranchPredictorWrapper::new(config),
            stat_paths: CorePaths::new(core_id),
        }
    }
}

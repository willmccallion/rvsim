//! The data prefetchers of the load/store unit and the L1D, modelled on
//! the Cortex-A72's load/store hardware prefetcher (TRM §6.4.9).

use serde::Deserialize;

/// Where a load prefetch stream stops at the end of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum PageBoundary {
    /// Stays inside the page of the load that trained the stream, at that
    /// page's size: the A72 with VA prefetch disabled (`CPUACTLR_EL1[43]`).
    #[default]
    Stop,
    /// Continues into a page whose translation the data TLB holds, and
    /// drops the prefetch when it does not: the A72's VA prefetch, its
    /// reset behaviour.
    CrossWithTlb,
}

/// The load/store unit's prefetcher for loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LoadPrefetcherConfig {
    /// No load prefetcher.
    #[default]
    None,
    /// A reference prediction table indexed by the load's PC, trained on
    /// virtual addresses, that keeps each confident stream `l1_lines` lines
    /// ahead in the L1D and `l2_lines` lines ahead in the L2.
    Stride {
        /// Table entries, a power of two.
        table_size: usize,
        /// Lines ahead of the demand stream prefetched into the L1D.
        l1_lines: usize,
        /// Lines ahead of the demand stream prefetched into the L2; the
        /// A72's `CPUECTLR_EL1[33:32]` resets to 22. No more than `l1_lines`
        /// sends nothing to the L2 alone.
        l2_lines: usize,
        /// Whether a stream crosses into the next page.
        page_boundary: PageBoundary,
    },
}

/// The L1D's prefetcher for store misses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StorePrefetcherConfig {
    /// No store prefetcher.
    #[default]
    None,
    /// Detects runs of store misses to adjacent lines in a 4 KiB physical
    /// page and prefetches the run `l2_lines` lines ahead into the L2 with
    /// write permission: the A72's PA-based store prefetcher, which
    /// prefetches only to the L2.
    Stream {
        /// Runs tracked at once.
        streams: usize,
        /// Lines ahead of the store misses prefetched into the L2.
        l2_lines: usize,
    },
}

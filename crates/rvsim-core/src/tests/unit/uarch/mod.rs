//! Microarchitecture: pipeline, branch prediction, caches, MMU, prefetchers.

pub mod bpred;
pub mod execution;
pub mod host_memory;
pub mod mmu;
pub mod pipeline;
pub mod prefetch;
pub mod pte_update;
pub mod watch;

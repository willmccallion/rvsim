//! Integration tests for the RISC-V emulator.

pub mod csr_ordering;
pub mod drain;
pub mod fetch_buffer;
pub mod fetch_inflight_limit;
pub mod fetch_page_crossing;
pub mod fetch_walk;
pub mod interrupts;
pub mod lsq_partial_overlap;
pub mod mmio_loads;
pub mod multicore;
pub mod page_crossing;
pub mod vector_pipeline;
pub mod tlb_latency;
pub mod xret_squash;
pub mod zicboz;

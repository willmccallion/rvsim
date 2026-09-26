//! Integration tests for the RISC-V emulator.

pub mod bus_occupancy;
pub mod csr_head_execution;
pub mod csr_ordering;
pub mod drain;
pub mod fault_precedence;
pub mod fetch_buffer;
pub mod fetch_inflight_limit;
pub mod fetch_line_straddle;
pub mod fetch_page_crossing;
pub mod fetch_walk;
pub mod fill_latency;
pub mod forwarding_latency;
pub mod frontend_prediction;
pub mod inorder_units;
pub mod interrupts;
pub mod line_crossing;
pub mod lsq_partial_overlap;
pub mod mmio_loads;
pub mod multicore;
pub mod page_crossing;
pub mod tlb_latency;
pub mod trap_latency;
pub mod vector_config;
pub mod vector_pipeline;
pub mod xret_squash;
pub mod zicboz;

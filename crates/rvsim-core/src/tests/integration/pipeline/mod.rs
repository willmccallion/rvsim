//! Pipeline behaviour: CSRs, traps, serialization, widths and stalls.

pub mod committed_prediction_stats;
pub mod csr_head_execution;
pub mod csr_ordering;
pub mod drain;
pub mod execute_trigger;
pub mod fault_precedence;
pub mod illegal_system;
pub mod inorder_units;
pub mod pipeline_stats;
pub mod rob_tag_wrap;
pub mod serialized_issue;
pub mod stage_widths;
pub mod trap_latency;
pub mod unit_status;
pub mod wfi_wake;
pub mod writeback_width;
pub mod xret_squash;

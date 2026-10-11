//! Running, stopping, checkpointing and controlling a whole simulation.

pub mod bare_metal;
pub mod checkpoint_resume;
#[cfg(feature = "commit-log")]
pub mod commit_log;
pub mod exit;
pub mod gdb;
pub mod host_files;
pub mod htif_syscalls;
pub mod isa_gating;
pub mod pipeline_trace;
pub mod pseudo_ops;
pub mod run_to;
pub mod se_mode;

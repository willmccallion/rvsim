//! Per-hart sim-side debug bookkeeping.
//!
//! Lives on the shared uncore state rather than `Hart`,
//! which is strict architectural state. These fields are simulator
//! observability of guest behaviour: hang detection, kernel-panic
//! observation, and a retired-instruction ring buffer used by the
//! invalid-PC dump.

/// Maximum number of `(pc, inst)` entries kept in the retire ring buffer.
pub const PC_TRACE_MAX: usize = 32;

/// Per-hart sim-only debug state.
#[derive(Debug)]
pub struct HartDebug {
    /// Last PC observed by the hang detector.
    pub last_pc: u64,
    /// Cycles the hart has spent at `last_pc` without progress.
    pub same_pc_count: u64,
    /// Ring buffer of the last [`PC_TRACE_MAX`] retired `(pc, inst)`
    /// pairs, consulted by the invalid-PC dump.
    pub pc_trace: Vec<(u64, u32)>,
}

impl Default for HartDebug {
    fn default() -> Self {
        Self {
            last_pc: 0,
            same_pc_count: 0,
            pc_trace: Vec::with_capacity(PC_TRACE_MAX),
        }
    }
}

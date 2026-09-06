//! Per-hart sim-side debug bookkeeping.
//!
//! Lives on the Simulator (or transitionally on SimState) rather than `Hart`,
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
    /// Cycle at which a kernel panic was first observed; the simulator
    /// keeps running for a short window so the full panic message can
    /// flush before exit.
    pub panic_detected_at_cycle: Option<u64>,
    /// Ring buffer of the last [`PC_TRACE_MAX`] retired `(pc, inst)`
    /// pairs, consulted by the invalid-PC dump.
    pub pc_trace: Vec<(u64, u32)>,
}

impl Default for HartDebug {
    fn default() -> Self {
        Self {
            last_pc: 0,
            same_pc_count: 0,
            panic_detected_at_cycle: None,
            pc_trace: Vec::with_capacity(PC_TRACE_MAX),
        }
    }
}

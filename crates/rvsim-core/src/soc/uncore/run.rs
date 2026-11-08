//! How a run is going: its exit, the guest's breaks and requests, and the
//! stats windows it asked for.

use crate::sim::stats::Stats;
use crate::soc::exit::ExitSignal;

/// The run's exit, breaks, pending guest requests and stats windows.
#[derive(Debug)]
pub struct RunControl {
    /// The run's exit code, which HTIF and `SysCon` signal into.
    pub exit_signal: ExitSignal,
    /// The cycle the guest's exit took effect at, while the run stays
    /// ended; the clock runs past it finishing the committed stores.
    pub exited_at: Option<u64>,
    /// Cycle at which a kernel panic was first observed; the simulator keeps
    /// running for a short window so the panic message can flush.
    pub panic_detected_at_cycle: Option<u64>,
    /// The label of a guest break the host has not yet stopped for.
    pub pending_break: Option<u64>,
    /// Requests retired pseudo-instructions made, applied at the end of
    /// the cycle.
    pub pending_sim_ops: Vec<SimOp>,
    /// Where the current stats window began.
    pub stats_epoch: StatsEpoch,
    /// Stats the guest dumped, oldest first.
    pub stats_dumps: Vec<StatsDump>,
}

impl RunControl {
    /// A run not yet started, that HTIF and `SysCon` end through `exit_signal`.
    #[must_use]
    pub fn new(exit_signal: ExitSignal) -> Self {
        Self {
            exit_signal,
            exited_at: None,
            panic_detected_at_cycle: None,
            pending_break: None,
            pending_sim_ops: Vec::new(),
            stats_epoch: StatsEpoch::default(),
            stats_dumps: Vec::new(),
        }
    }
}

/// A request from the guest that the simulator carries out at the end
/// of the cycle the pseudo-instruction asking for it retires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimOp {
    /// Zero every stat and start counting from here.
    ResetStats,
    /// Keep a copy of the stats, labelled.
    DumpStats {
        /// The guest's label for this dump.
        label: u64,
    },
    /// Stop the host's run here, so it can inspect, save or switch.
    Break {
        /// The guest's label for this point.
        label: u64,
    },
}

/// The cycle and instructions retired when the stats were last reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatsEpoch {
    /// The cycle of the reset.
    pub cycle: u64,
    /// Instructions all harts had retired by then.
    pub instructions_retired: u64,
}

/// A copy of the stats a guest asked for. Two dumps with the same epoch
/// subtract to the stats of the region between them.
#[derive(Clone, Debug)]
pub struct StatsDump {
    /// The guest's label.
    pub label: u64,
    /// The reset the counts start from.
    pub epoch: StatsEpoch,
    /// The stats since the last reset.
    pub stats: Stats,
    /// Cycles since the last reset.
    pub cycles: u64,
    /// Instructions retired since the last reset.
    pub instructions_retired: u64,
}

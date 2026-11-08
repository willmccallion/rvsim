//! What every timed component runs against: the clock, the event queue,
//! the statistics, the memory image and the configuration.

use crate::config::Config;
use crate::sim::components::ComponentId;
use crate::sim::events::EventQueue;
use crate::sim::handle::HandleCtx;
use crate::sim::memory::GlobalMemory;
use crate::sim::stats::Stats;

/// The simulation kernel the uncore's components are dispatched through.
#[derive(Debug)]
pub struct SimKernel {
    /// Master clock; every subsystem reads from this.
    pub cycle: u64,
    /// Global event queue: every inter-component message lands here.
    pub event_queue: EventQueue,
    /// Hierarchical statistics tree; sim-side perf observability counters.
    pub stats: Stats,
    /// RAM with the LR/SC reservations and the log of RAM writes.
    pub memory: GlobalMemory,
    /// Simulator parameters (cache sizes, ISA capability flags, pipeline
    /// knobs, system layout).
    pub config: Config,
}

impl SimKernel {
    /// The context component `self_id` handles an event or ticks in.
    #[inline]
    pub const fn ctx(&mut self, self_id: ComponentId) -> HandleCtx<'_> {
        HandleCtx {
            scheduler: &mut self.event_queue,
            stats: &mut self.stats,
            memory: &mut self.memory,
            config: &self.config,
            cycle: self.cycle,
            self_id,
        }
    }
}

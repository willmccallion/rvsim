//! Everything outside the cores that the pipelines share.
//!
//! The [`Uncore`] holds the bus and its devices, the memory controller,
//! the shared LLC, the coherence fabric and RAM, together with the
//! simulator's clock, event queue, stats and run control.

pub mod debug;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::common::{HartId, PhysAddr, SimError};
use crate::config::Config;
use crate::isa::op::MemWidth;
use crate::isa::privileged::Trap;
use crate::sim::components::CacheId;
use crate::sim::events::EventQueue;
use crate::sim::memory::GlobalMemory;
use crate::sim::memory::write_log::Writer;
use crate::sim::stats::Stats;
use crate::sim::stats::paths::HartPaths;
use crate::soc::bus::Bus;
use crate::soc::cache::Cache;
use crate::soc::coherence::CoherenceFabric;
use crate::soc::devices::Htif;
use crate::soc::memory::controller::MemoryController;
use crate::soc::topology::Topology;
use debug::HartDebug;

/// What the trace macros print once tracing is armed.
///
/// Every event, or only those of some harts, inside a cycle window, or for
/// some trap causes. The simulator resolves this into the per-core switch
/// every component reads (`config.general.trace_instructions`) before each
/// core's tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceControl {
    /// Tracing is switched on at all.
    pub armed: bool,
    /// Harts whose events print; empty means every hart.
    pub harts: Vec<HartId>,
    /// First cycle that prints, when set.
    pub cycle_from: Option<u64>,
    /// Last cycle that prints, when set.
    pub cycle_to: Option<u64>,
    /// `mcause` values (interrupt bit included) whose trap-taken events
    /// print; empty means every trap except the timer and ecall traffic
    /// that would swamp a trace.
    pub trap_causes: Vec<u64>,
}

impl TraceControl {
    /// Whether events of `hart` print at `cycle`.
    #[must_use]
    pub fn applies(&self, hart: Option<HartId>, cycle: u64) -> bool {
        self.armed
            && self.cycle_from.is_none_or(|from| cycle >= from)
            && self.cycle_to.is_none_or(|to| cycle <= to)
            && hart.is_none_or(|h| self.harts.is_empty() || self.harts.contains(&h))
    }

    /// Whether a trap with `mcause` value `code` prints; `routine` marks
    /// the timer and ecall traffic that is hidden unless asked for.
    #[must_use]
    pub fn trap_visible(&self, code: u64, routine: bool) -> bool {
        if self.trap_causes.is_empty() { !routine } else { self.trap_causes.contains(&code) }
    }
}

/// The uncore: everything shared by all cores.
#[derive(Debug)]
pub struct Uncore {
    /// What the trace macros print; see [`TraceControl`].
    pub trace: TraceControl,
    /// Component identifiers for the whole system.
    pub topology: Topology,
    /// Master clock; every subsystem reads from this.
    pub cycle: u64,
    /// IO interconnect; routes accesses to RAM and MMIO devices.
    pub bus: Bus,
    /// Main memory controller.
    pub mem_controller: Box<dyn MemoryController + Send + Sync>,
    /// Shared last-level cache.
    pub l3_cache: Cache,
    /// Home agent and interconnect between the private L2s and the LLC;
    /// present only when more than one core shares memory.
    pub coherence: Option<CoherenceFabric>,
    /// RAM with the LR/SC reservations and the log of RAM writes.
    pub memory: GlobalMemory,
    /// Simulator parameters (cache sizes, ISA capability flags, pipeline
    /// knobs, system layout).
    pub config: Config,
    /// Sim-side per-hart debug bookkeeping (hang detection, retire trace),
    /// indexed by `HartId`.
    pub per_hart_debug: Vec<HartDebug>,
    /// Cycle at which a kernel panic was first observed; the simulator keeps
    /// running for a short window so the panic message can flush.
    pub panic_detected_at_cycle: Option<u64>,
    /// Optional buffered writer for the commit log (enabled by the
    /// `commit-log` feature).
    #[cfg(feature = "commit-log")]
    pub commit_log: Option<std::io::BufWriter<std::fs::File>>,
    /// Atomic slot bus-resident devices (HTIF, `SysCon`) write the harness
    /// termination value into. `u64::MAX` means "no exit pending".
    pub exit_signal: Arc<AtomicU64>,
    /// Latched termination value the harness returns from `take_exit`.
    pub exit_code: Option<u64>,
    /// Direct mode (no translation, flat memory). Initialised from
    /// `config.general.direct_mode` and runtime-mutable (the ELF loader
    /// writes it after init).
    pub direct_mode: bool,
    /// Global event queue: every inter-component message lands here.
    pub event_queue: EventQueue,
    /// Hierarchical statistics tree; sim-side perf observability counters.
    pub stats: Stats,
    /// Where the current stats window began.
    pub stats_epoch: StatsEpoch,
    /// Stats the guest dumped, oldest first.
    pub stats_dumps: Vec<StatsDump>,
    /// The label of a guest break the host has not yet stopped for.
    pub pending_break: Option<u64>,
    /// Stat paths rooted at `hart<N>`, indexed by `HartId`.
    pub hart_stat_paths: Vec<HartPaths>,
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

impl Uncore {
    /// Makes `data` visible at `paddr` as an immediate write by `writer`,
    /// for writes that do not travel through the memory system: an MMIO
    /// address is left to the device that receives the packet.
    pub fn publish_write(&mut self, writer: Writer, paddr: PhysAddr, data: u64, width: MemWidth) {
        let width_bytes = width.bytes();
        if width_bytes == 0 || self.bus.ram_region_for(paddr.val(), width_bytes).is_none() {
            return;
        }
        self.memory.write(writer, paddr, data, width_bytes as usize);
    }

    /// Whether a trace event about `trap` prints under the current
    /// [`TraceControl`] settings.
    #[must_use]
    pub fn trace_trap_enabled(&self, trap: &Trap) -> bool {
        self.config.general.trace_instructions
            && self.trace.trap_visible(trap.mcause_code(), trap.is_routine())
    }

    /// Atomically takes the exit code if a bus device has signalled termination.
    pub fn take_exit(&self) -> Option<u64> {
        let val = self.exit_signal.swap(u64::MAX, Ordering::Relaxed);
        if val == u64::MAX { None } else { Some(val) }
    }

    /// Returns the exit code if a device has requested shutdown without
    /// clearing the signal slot.
    pub fn check_exit(&self) -> Option<u64> {
        let val = self.exit_signal.load(Ordering::Relaxed);
        if val == u64::MAX { None } else { Some(val) }
    }

    /// Manually signals exit (used by tests and the binding `step` loop).
    pub fn signal_exit(&self, code: u64) {
        self.exit_signal.store(code, Ordering::Relaxed);
    }

    /// Registers an HTIF device at the given tohost address. `exit_signal`
    /// is cloned into the device so HTIF tohost writes propagate up to the
    /// harness.
    pub fn add_htif(&mut self, tohost_addr: u64, exit_signal: &Arc<AtomicU64>) {
        let htif = Htif::new(tohost_addr, exit_signal.clone());
        self.bus.add_device(Box::new(htif));
    }

    /// Loads a binary into memory at the given physical address.
    pub const fn load_binary_at(&mut self, data: &[u8], addr: PhysAddr) {
        self.bus.load_binary_at(data, addr);
    }

    /// Returns the `CacheId` of the shared LLC.
    #[must_use]
    pub const fn l3_cache_id(&self) -> CacheId {
        self.topology.llc
    }

    /// Opens a commit log file for writing retired instruction traces.
    ///
    /// Each retired instruction is logged as `core   0: 0x<pc> (0x<inst>)\n`.
    /// Only available when the `commit-log` Cargo feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::FileRead`](crate::common::SimError::FileRead) if
    /// the file cannot be created.
    #[cfg(feature = "commit-log")]
    pub fn open_commit_log(&mut self, path: &str) -> Result<(), crate::common::SimError> {
        use std::fs::File;
        use std::io::BufWriter;
        let file = File::create(path).map_err(|source| crate::common::SimError::FileRead {
            path: path.to_owned(),
            source,
        })?;
        self.commit_log = Some(BufWriter::with_capacity(1 << 20, file));
        Ok(())
    }

    /// Uncore work at the top of a cycle: exit and kernel-panic checks, then
    /// one tick of every bus device, which samples every hart's interrupt
    /// lines. Returns `false` when a device has requested exit and the
    /// cycle should be skipped.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::KernelPanic`] when the bus panic sentinel fires.
    pub fn pre_cycle(&mut self) -> Result<bool, SimError> {
        if self.check_exit().is_some() {
            return Ok(false);
        }

        if self.bus.check_kernel_panic() {
            let detected_at = *self.panic_detected_at_cycle.get_or_insert(self.cycle);
            if self.cycle.saturating_sub(detected_at) >= 10_000 {
                return Err(SimError::KernelPanic { cycle: detected_at });
            }
        }

        self.bus.tick();
        Ok(true)
    }

    /// Advances the master clock by one cycle.
    pub const fn advance_cycle(&mut self) {
        self.cycle += 1;
    }
}

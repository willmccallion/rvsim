//! System state: every hart's architectural state, every core's private
//! micro-architecture, and the shared uncore.
//!
//! `SimState` owns the whole system. Pipelines never see it; they work on a
//! [`CoreCtx`] view that borrows one hart, one core and the shared uncore,
//! so a core cannot reach another core's state by construction. See
//! `docs/architecture/multicore.md`.

/// Control and Status Register access and management.
pub mod csr;

/// Per-cycle hart bookkeeping (interrupts, hang detection, mode tracing).
pub mod execution;

/// RAM, reservations and write log: what every access takes effect against.
pub mod global_memory;

/// Address translation for the pipeline.
pub mod memory;

/// LR/SC reservations shared by all harts.
pub mod reservations;

/// Trap and exception handling logic.
pub mod trap;

/// The restricted view every stage but commit works on.
pub mod views;

/// Record of RAM writes for cross-hart visibility checks.
pub mod write_log;

use crate::coherence::{self, CoherenceFabric, FabricGeometry};
use crate::common::{HartId, PhysAddr, RegisterFile, Trap};
use crate::config::{Config, InclusionPolicy, MemoryController as MemControllerType};
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::exec::signals::MemWidth;
use crate::core::hart::HartInit;
use crate::core::pipeline::engine::PipelineDispatch;
use crate::core::units::cache::Cache;
use crate::core::units::mmu::pmp::Pmp;
use crate::core::{Core, CoreUnits, Hart};
use crate::sim::components::{CacheId, ComponentId, MemCtrlId};
use crate::sim::events::EventQueue;
use crate::sim::packet::CacheLevel;
use crate::sim::per_hart_debug::HartDebug;
use crate::sim::stats::Stats;
use crate::sim::stats::paths::HartPaths;
use crate::sim::topology::Topology;
use crate::soc::devices::{
    Clint, GoldfishRtc, Htif, Plic, SimControl, SimOp, SysCon, Uart, VirtioBlock,
};
use crate::soc::interconnect::Bus;
use crate::soc::memory::buffer::DramBuffer;
use crate::soc::memory::controller::{
    Bandwidth, DramConfig, DramController, MemoryController, SimpleController,
};
use crate::soc::memory::ddr5::Ddr5Controller;
use global_memory::GlobalMemory;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use write_log::Writer;

pub use views::StageCtx;

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
pub struct SharedState {
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

/// The whole system: harts, cores, and the uncore.
#[derive(Debug)]
pub struct SimState {
    /// Architectural state per hardware thread, indexed by `HartId`.
    pub harts: Vec<Hart>,
    /// Private micro-architecture per core, indexed by `CoreId`.
    pub cores: Vec<Core>,
    /// The uncore.
    pub shared: SharedState,
}

unsafe impl Send for SimState {}
unsafe impl Sync for SimState {}

impl Deref for SimState {
    type Target = SharedState;

    fn deref(&self) -> &SharedState {
        &self.shared
    }
}

impl DerefMut for SimState {
    fn deref_mut(&mut self) -> &mut SharedState {
        &mut self.shared
    }
}

/// What a pipeline works on: its hart, its core, and the uncore.
///
/// Built by [`SimState::core_ctx`] from disjoint borrows. Derefs to
/// [`SharedState`] so uncore fields read as `ctx.bus`, `ctx.event_queue`.
#[derive(Debug)]
pub struct CoreCtx<'a> {
    /// The hart the pipeline is executing.
    pub hart: &'a mut Hart,
    /// The pipeline's private micro-architecture.
    pub core: &'a mut CoreUnits,
    /// The uncore.
    pub shared: &'a mut SharedState,
}

impl Deref for CoreCtx<'_> {
    type Target = SharedState;

    fn deref(&self) -> &SharedState {
        self.shared
    }
}

impl DerefMut for CoreCtx<'_> {
    fn deref_mut(&mut self) -> &mut SharedState {
        self.shared
    }
}

impl CoreCtx<'_> {
    /// The view a stage other than commit works on: the hart read-only,
    /// the core and the uncore's stats and event queue mutable.
    #[inline]
    pub const fn stage(&mut self) -> StageCtx<'_> {
        StageCtx::new(self.hart, self.core, self.shared)
    }

    /// Stat paths of the hart this view executes.
    #[inline]
    #[must_use]
    pub fn hart_paths(&self) -> HartPaths {
        self.shared.hart_stat_paths[self.hart.hart_id.as_index()]
    }

    /// Sets a load reservation for this hart at `addr` (cache-line aligned).
    #[inline]
    pub fn set_reservation(&mut self, addr: PhysAddr) {
        let hart = self.hart.hart_id;
        self.shared.memory.reservations_mut().set(hart, addr);
    }

    /// Returns `true` when this hart holds a reservation covering `addr`.
    #[inline]
    pub fn check_reservation(&self, addr: PhysAddr) -> bool {
        self.shared.memory.reservations().check(self.hart.hart_id, addr)
    }

    /// Clears this hart's load reservation.
    #[inline]
    pub fn clear_reservation(&mut self) {
        let hart = self.hart.hart_id;
        self.shared.memory.reservations_mut().clear(hart);
    }

    /// Makes `data` visible at `paddr` as a write by this hart. See
    /// [`SharedState::publish_write`].
    #[inline]
    pub fn publish_write(&mut self, paddr: PhysAddr, data: u64, width: MemWidth) {
        let writer = Writer::Hart(self.hart.hart_id);
        self.shared.publish_write(writer, paddr, data, width);
    }
}

impl SharedState {
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
}

impl SimState {
    /// The execution view for core `core`: its first hart, its private
    /// micro-architecture, and the uncore.
    ///
    /// # Panics
    ///
    /// Panics if `core` is not a valid core index.
    pub fn core_ctx(&mut self, core: usize) -> CoreCtx<'_> {
        let hart_index = self.shared.topology.cores[core].hart_ids[0].as_index();
        CoreCtx {
            hart: &mut self.harts[hart_index],
            core: &mut self.cores[core].units,
            shared: &mut self.shared,
        }
    }

    /// Core `core`'s pipeline together with the execution view it runs in.
    ///
    /// # Panics
    ///
    /// Panics if `core` is not a valid core index.
    pub fn pipeline_ctx(&mut self, core: usize) -> (&mut PipelineDispatch, CoreCtx<'_>) {
        let hart_index = self.shared.topology.cores[core].hart_ids[0].as_index();
        let Core { units, pipeline } = &mut self.cores[core];
        let ctx =
            CoreCtx { hart: &mut self.harts[hart_index], core: units, shared: &mut self.shared };
        (pipeline, ctx)
    }

    /// Instructions retired across every hart.
    pub fn instructions_retired(&self) -> u64 {
        self.harts.iter().map(|h| h.instructions_retired).sum()
    }

    /// Cycles and instructions retired since the stats were last reset.
    #[must_use]
    pub fn stats_window(&self) -> (u64, u64) {
        let epoch = self.stats_epoch;
        (self.cycle - epoch.cycle, self.instructions_retired() - epoch.instructions_retired)
    }

    /// Zeroes every stat and starts a new window here.
    pub fn reset_stats(&mut self) {
        self.shared.stats.reset();
        self.shared.stats_epoch =
            StatsEpoch { cycle: self.cycle, instructions_retired: self.instructions_retired() };
    }

    /// Carries out a request the guest made through the sim-control device.
    pub fn apply_sim_op(&mut self, op: SimOp) {
        match op {
            SimOp::ResetStats => self.reset_stats(),
            SimOp::DumpStats { label } => {
                let (cycles, instructions_retired) = self.stats_window();
                let stats = self.shared.stats.clone();
                let epoch = self.shared.stats_epoch;
                self.shared.stats_dumps.push(StatsDump {
                    label,
                    epoch,
                    stats,
                    cycles,
                    instructions_retired,
                });
            }
            SimOp::Break { label } => self.shared.pending_break = Some(label),
        }
    }

    /// Dumps every hart's state (PC and registers) to stdout.
    pub fn dump_state(&self) {
        for hart in &self.harts {
            hart.dump_state();
        }
    }

    /// Convenience constructor: allocates a fresh `exit_signal` slot and
    /// builds the system. Use this when the caller doesn't need to share the
    /// signal `Arc` with other components before construction.
    pub fn build(config: &Config, disk_path: &str) -> Self {
        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        Self::new(config, disk_path, exit_signal)
    }

    /// Constructs the system: bus and devices, memory controller, LLC, and
    /// `config.system.hart_count` cores each hosting one hart. `exit_signal`
    /// is cloned into bus-resident devices (`SysCon`, HTIF) so they can
    /// write the harness termination value when triggered.
    pub fn new(config: &Config, disk_path: &str, exit_signal: Arc<AtomicU64>) -> Self {
        use crate::core::arch::csr::{MSTATUS_DEFAULT_RV64, MSTATUS_FS_INIT, MSTATUS_VS_INIT};
        use crate::isa::abi;

        let topology = Topology::single_threaded_cores(config.system.hart_count.max(1));
        let hart_count = topology.hart_count();

        // --- Bus + devices ---------------------------------------------
        let mut bus = Bus::new(config.system.bus_width, config.system.bus_latency, hart_count);

        let ram_base = config.system.ram_base;
        let ram_size = config.memory.ram_size;
        let ram_buffer = Arc::new(DramBuffer::new(ram_size));

        let uart =
            Uart::new(config.system.uart_base, config.system.console, config.system.cpu_clock_mhz);
        let clint = Clint::new(config.system.clint_base, config.system.clint_divider, hart_count);
        let plic = Plic::new(0x0c00_0000, hart_count);

        let mut disk = VirtioBlock::new(config.system.disk_base, ram_base, ram_buffer.clone());
        if !disk_path.is_empty()
            && let Ok(disk_data) = fs::read(disk_path)
            && !disk_data.is_empty()
        {
            disk.load(disk_data);
        }

        let syscon = SysCon::new(config.system.syscon_base, exit_signal.clone());
        let rtc = GoldfishRtc::new(
            0x101000,
            config.system.rtc_epoch_seconds.saturating_mul(1_000_000_000),
            config.system.cpu_clock_mhz,
        );

        bus.add_device(Box::new(uart));
        bus.add_device(Box::new(disk));
        bus.add_device(Box::new(clint));
        bus.add_device(Box::new(plic));
        bus.add_device(Box::new(syscon));
        bus.add_device(Box::new(SimControl::new(
            config.system.sim_control_base,
            exit_signal.clone(),
        )));
        bus.add_device(Box::new(rtc));

        if config.system.tohost_addr != 0 {
            let htif = Htif::new(config.system.tohost_addr, exit_signal.clone());
            bus.add_device(Box::new(htif));
        }

        let mem_controller: Box<dyn MemoryController + Send + Sync> = match config.memory.controller
        {
            MemControllerType::Dram => Box::new(DramController::new(
                ram_buffer.clone(),
                PhysAddr::new(ram_base),
                DramConfig {
                    t_cas: config.memory.t_cas,
                    t_ras: config.memory.t_ras,
                    t_pre: config.memory.t_pre,
                    t_rrd: config.memory.t_rrd,
                    num_banks: config.memory.num_banks,
                    row_size_bytes: config.memory.row_size_bytes,
                    t_refi: config.memory.t_refi,
                    t_rfc: config.memory.t_rfc,
                },
            )),
            MemControllerType::Simple => {
                let bytes_per_second = config
                    .memory
                    .simple_bandwidth_bytes_per_second()
                    .unwrap_or(std::num::NonZeroU64::MAX);
                Box::new(SimpleController::new(
                    ram_buffer.clone(),
                    PhysAddr::new(ram_base),
                    config.memory.row_miss_latency,
                    Bandwidth::new(bytes_per_second, config.system.cpu_clock_mhz * 1_000_000),
                ))
            }
            MemControllerType::Ddr5 => Box::new(Ddr5Controller::new(
                ram_buffer.clone(),
                PhysAddr::new(ram_base),
                config.memory.ddr5.to_config(),
                MemCtrlId::new(0),
                config.system.cpu_clock_mhz,
            )),
        };

        let mut l3_cache = Cache::new(topology.llc, CacheLevel::L3, &config.cache.l3, "llc");
        l3_cache.set_downstream(ComponentId::Bus);
        // An exclusive L1/L2 pair leaves the LLC non-inclusive of the L2s.
        let llc_inclusion = match config.cache.inclusion_policy {
            InclusionPolicy::Exclusive => InclusionPolicy::Nine,
            policy => policy,
        };
        l3_cache.set_upstream_inclusion(llc_inclusion);

        let ram_region =
            crate::soc::memory::RamRegion::new(ram_buffer.as_mut_ptr(), ram_base, ram_size as u64);
        bus.attach_ram(MemCtrlId::new(0), ram_region);
        let write_log_line_bytes = match config.cache.l1_d.line_bytes {
            0 => 64,
            line_bytes => line_bytes as u64,
        };

        // --- Hart architectural state ----------------------------------
        let configured_misa = config.misa().bits();

        let direct_mode = config.general.direct_mode;

        // In direct (SE) mode, enable FP state so user programs can use
        // floating-point instructions without an OS to set mstatus.FS/VS.
        // In full-system mode, firmware/OS is responsible for enabling FP/V.
        let mstatus = if direct_mode {
            MSTATUS_DEFAULT_RV64 | MSTATUS_FS_INIT | MSTATUS_VS_INIT
        } else {
            MSTATUS_DEFAULT_RV64
        };

        let vlenb = config.pipeline.vlen / 8;
        let csrs = Csrs {
            mstatus,
            misa: configured_misa,
            stimecmp: u64::MAX,
            vlenb: vlenb as u64,
            ..Default::default()
        };

        // Direct-mode programs get the bare-metal boot convention: a0 = hart
        // id, a1 = hart count, sp = a stack top every hart shares (a
        // multi-hart runtime carves per-hart stacks below it).
        let fresh_regs = |hart_id: HartId| {
            let mut regs = if direct_mode {
                let sp = config.general.initial_sp.unwrap_or(config.system.ram_base + 0x100_0000);
                let mut r = RegisterFile::new();
                r.write(abi::REG_SP, sp);
                r.write(abi::REG_A0, u64::from(hart_id.val()));
                r.write(abi::REG_A1, hart_count as u64);
                r
            } else {
                RegisterFile::new()
            };
            // Initialize vector register file if VLEN > 0
            if config.pipeline.vlen > 0
                && let Ok(vlen) = crate::isa::vector::Vlen::new(config.pipeline.vlen)
            {
                regs.init_vpr(vlen);
            }
            regs
        };

        // Always start in Machine mode. The riscv-tests switch to lower modes
        // via their own trap handlers; bare-metal binaries need M-mode too.
        let privilege = PrivilegeMode::Machine;

        let harts: Vec<Hart> = (0..hart_count)
            .map(|index| {
                let hart_id = HartId::new(u32::try_from(index).unwrap_or(u32::MAX));
                Hart::new(HartInit {
                    hart_id,
                    regs: fresh_regs(hart_id),
                    pc: config.general.start_pc,
                    csrs: csrs.clone(),
                    privilege,
                    pmp: Pmp::new(),
                })
            })
            .collect();

        let mut units: Vec<CoreUnits> = topology
            .cores
            .iter()
            .map(|c| CoreUnits::new(c.core_id, config, c.l1i.val(), topology.llc))
            .collect();
        let coherence = if units.len() > 1 {
            Some(Self::attach_coherence_fabric(config, &mut units, &mut l3_cache))
        } else {
            for core in &units {
                l3_cache.add_upstream(ComponentId::Cache(core.l2_cache.id));
            }
            None
        };

        let hart_stat_paths: Vec<HartPaths> =
            harts.iter().map(|hart| HartPaths::new(hart.hart_id)).collect();
        let core_stat_paths: Vec<_> = topology
            .cores
            .iter()
            .zip(&units)
            .map(|(c, core)| (core.stat_paths, c.hart_ids[0]))
            .collect();
        let cache_stat_paths: Vec<_> = units
            .iter()
            .flat_map(|core| {
                [core.l1_i_cache.stat_paths, core.l1_d_cache.stat_paths, core.l2_cache.stat_paths]
            })
            .chain(std::iter::once(l3_cache.stat_paths))
            .collect();
        let stats = Stats::for_components(
            &hart_stat_paths,
            &core_stat_paths,
            &cache_stat_paths,
            coherence.as_ref().map(CoherenceFabric::stat_paths),
        );

        let cores = topology
            .cores
            .iter()
            .zip(units)
            .map(|(c, units)| Core {
                units,
                pipeline: PipelineDispatch::new(config, c, config.general.start_pc),
            })
            .collect();

        Self {
            harts,
            cores,
            shared: SharedState {
                trace: TraceControl {
                    armed: config.general.trace_instructions,
                    ..TraceControl::default()
                },
                topology,
                cycle: 0,
                bus,
                mem_controller,
                l3_cache,
                coherence,
                memory: GlobalMemory::new(Some(ram_region), hart_count, write_log_line_bytes),
                config: config.clone(),
                per_hart_debug: (0..hart_count).map(|_| HartDebug::default()).collect(),
                panic_detected_at_cycle: None,
                #[cfg(feature = "commit-log")]
                commit_log: None,
                exit_signal,
                exit_code: None,
                direct_mode,
                event_queue: EventQueue::new(),
                stats,
                stats_epoch: StatsEpoch::default(),
                stats_dumps: Vec::new(),
                pending_break: None,
                hart_stat_paths,
            },
        }
    }

    /// Puts the coherence fabric between every core's L2 and the LLC: the
    /// L2s become requesting agents (inclusive of their L1s so a snoop can
    /// be answered from their tags) and the LLC serves the home agent. A
    /// core with no private cache at all holds no lines, so its accesses
    /// cross the fabric without taking part in coherence.
    fn attach_coherence_fabric(
        config: &Config,
        cores: &mut [CoreUnits],
        llc: &mut Cache,
    ) -> CoherenceFabric {
        let agents: Vec<ComponentId> =
            cores.iter().map(|core| ComponentId::Cache(core.l2_cache.id)).collect();
        let caches_lines =
            config.cache.l1_i.enabled || config.cache.l1_d.enabled || config.cache.l2.enabled;
        for core in cores.iter_mut() {
            core.l2_cache.set_downstream(ComponentId::Fabric);
            if caches_lines {
                core.l2_cache.set_coherent(core.core_id);
            }
            core.l2_cache.set_upstream_inclusion(InclusionPolicy::Inclusive);
        }
        llc.add_upstream(ComponentId::Fabric);
        llc.set_upstream_inclusion(InclusionPolicy::Nine);
        let line_bytes = llc.line_bytes();
        let private_l2_lines = cores.iter().map(|core| core.l2_cache.line_count()).sum();
        coherence::build(
            &config.coherence,
            FabricGeometry { line_bytes, private_l2_lines },
            ComponentId::Cache(llc.id),
            agents,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_cpu_reservation() {
        let config = Config::default();
        let mut sys = SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.set_reservation(PhysAddr::new(0x1000));
        assert!(state.check_reservation(PhysAddr::new(0x1000)));
        assert!(state.check_reservation(PhysAddr::new(0x1008)));
        assert!(!state.check_reservation(PhysAddr::new(0x2000)));

        state.clear_reservation();
        assert!(!state.check_reservation(PhysAddr::new(0x1000)));
    }

    #[test]
    fn test_cpu_dump_state_no_panic() {
        let config = Config::default();
        let state = SimState::build(&config, "");
        state.dump_state();
    }

    #[test]
    fn test_cpu_take_exit() {
        let config = Config::default();
        let state = SimState::build(&config, "");

        assert_eq!(state.take_exit(), None);
        state.signal_exit(42);
        assert_eq!(state.take_exit(), Some(42));
        assert_eq!(state.take_exit(), None);
    }

    #[test]
    fn direct_mode_harts_boot_with_their_id_and_the_hart_count() {
        use crate::isa::abi;
        let mut config = Config::default();
        config.general.direct_mode = true;
        config.system.hart_count = 3;
        let sys = SimState::build(&config, "");
        for (index, hart) in sys.harts.iter().enumerate() {
            assert_eq!(hart.regs.read(abi::REG_A0), index as u64);
            assert_eq!(hart.regs.read(abi::REG_A1), 3);
            assert_eq!(hart.regs.read(abi::REG_SP), config.system.ram_base + 0x100_0000);
        }
    }

    #[test]
    fn every_core_gets_its_own_hart_and_caches() {
        let mut config = Config::default();
        config.system.hart_count = 2;
        let sys = SimState::build(&config, "");
        assert_eq!(sys.harts.len(), 2);
        assert_eq!(sys.cores.len(), 2);
        assert_eq!(sys.harts[1].hart_id, HartId::new(1));
        assert_eq!(sys.cores[1].units.core_id, crate::common::CoreId::new(1));
        assert_ne!(sys.cores[0].units.l2_cache.id, sys.cores[1].units.l2_cache.id);
        assert_eq!(sys.l3_cache.id, sys.topology.llc);
    }
}

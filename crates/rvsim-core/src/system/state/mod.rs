//! System state: every hart's architectural state, every core's private
//! micro-architecture, and the shared uncore.
//!
//! `SystemState` owns the whole system. Pipelines never see it; they work on a
//! [`CoreCtx`] view that borrows one hart, one core and the shared uncore,
//! so a core cannot reach another core's state by construction. See
//! `docs/architecture/multicore.md`.

use crate::arch::csr::Csrs;
use crate::arch::pmp::Pmp;
use crate::arch::regs::RegisterFile;
use crate::arch::{Hart, HartInit};
use crate::common::{HartId, NANOS_PER_SECOND, SimError, VirtAddr};
use crate::config::{Config, InclusionPolicy, MemoryControllerKind, ValidConfig};
use crate::isa::csr::MSTATUS_DEFAULT_RV64;
use crate::isa::privileged::PrivilegeMode;
use crate::sim::components::{ComponentId, MemCtrlId};
use crate::sim::events::EventQueue;
use crate::sim::kernel::SimKernel;
use crate::sim::memory::{GlobalMemory, Ram};
use crate::sim::packet::CacheLevel;
use crate::sim::stats::paths::HartPaths;
use crate::sim::stats::{StatSource, Stats};
use crate::soc::bus::Bus;
use crate::soc::cache::Cache;
use crate::soc::coherence::{self, CoherenceFabric, FabricGeometry};
use crate::soc::devices::{Clint, GoldfishRtc, Htif, Plic, SysCon, Uart, VirtioBlock};
use crate::soc::exit::ExitSignal;
use crate::soc::memory::controller::{
    Bandwidth, DramConfig, DramController, MemoryController, SimpleController,
};
use crate::soc::memory::ddr5::Ddr5Controller;
use crate::soc::terminal::{SharedTerminal, Terminal};
use crate::soc::topology::Topology;
use crate::soc::uncore::debug::HartDebug;
use crate::soc::uncore::{
    HostSide, RunControl, SimOp, StatsDump, StatsEpoch, TraceControl, Uncore,
};
use crate::system::loader::load_binary;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::engine::PipelineDispatch;
use crate::uarch::{Core, CoreUnits};

/// The whole system: harts, cores, and the uncore.
#[derive(Debug)]
pub struct SystemState {
    /// Architectural state per hardware thread, indexed by `HartId`.
    pub harts: Vec<Hart>,
    /// Private micro-architecture per core, indexed by `CoreId`.
    pub cores: Vec<Core>,
    /// The uncore.
    pub uncore: Uncore,
}

/// The bus with RAM and every device on it: the console UART, the disk
/// (with `disk`'s image), the CLINT, PLIC, `SysCon`, RTC and, for a
/// program that has one, HTIF.
fn build_bus(
    config: &Config,
    hart_count: usize,
    terminal: &SharedTerminal,
    exit_signal: &ExitSignal,
    disk: Option<&str>,
) -> Result<Bus, SimError> {
    let system = &config.system;
    let mut bus = Bus::new(system.bus_width, system.bus_latency, hart_count);
    let mut disk_device = VirtioBlock::new(system.disk_base.val());
    if let Some(path) = disk {
        disk_device.load(load_binary(path)?);
    }
    let rtc_epoch_ns = system.rtc_epoch_seconds.saturating_mul(NANOS_PER_SECOND);
    bus.add_device(Box::new(Uart::new(
        system.uart_base.val(),
        terminal.clone(),
        system.cpu_clock_mhz,
    )));
    bus.add_device(Box::new(disk_device));
    bus.add_device(Box::new(Clint::new(
        system.clint_base.val(),
        system.clint_divider(),
        hart_count,
    )));
    bus.add_device(Box::new(Plic::new(crate::config::PLIC_BASE, hart_count)));
    bus.add_device(Box::new(SysCon::new(system.syscon_base.val(), exit_signal.clone())));
    bus.add_device(Box::new(GoldfishRtc::new(
        crate::config::RTC_BASE,
        rtc_epoch_ns,
        system.cpu_clock_mhz,
    )));
    if let Some(tohost) = system.tohost_addr {
        let htif = Htif::new(tohost.val(), exit_signal.clone(), Vec::new(), terminal.clone());
        bus.add_device(Box::new(htif));
    }
    bus.attach_ram(MemCtrlId::new(0), system.ram_base.val(), config.memory.ram_size as u64);
    Ok(bus)
}

/// The memory controller the configuration picks.
fn build_mem_controller(config: &Config) -> Box<dyn MemoryController> {
    match config.memory.controller {
        MemoryControllerKind::Dram => {
            Box::new(DramController::new(DramConfig::from(&config.memory)))
        }
        MemoryControllerKind::Simple => {
            let bytes_per_second = config
                .memory
                .simple_bandwidth_bytes_per_second()
                .unwrap_or(std::num::NonZeroU64::MAX);
            Box::new(SimpleController::new(
                config.memory.simple_latency,
                Bandwidth::new(bytes_per_second, config.system.cpu_clock_hz()),
            ))
        }
        MemoryControllerKind::Ddr5 => Box::new(Ddr5Controller::new(
            config.system.ram_base,
            config.memory.ram_size as u64,
            &config.memory.ddr5.to_config(),
            MemCtrlId::new(0),
            config.system.cpu_clock_mhz,
        )),
    }
}

/// The shared LLC, in front of the bus.
fn build_llc(config: &Config, topology: &Topology) -> Cache {
    let mut llc = Cache::new(topology.llc, CacheLevel::L3, &config.cache.l3, "llc");
    llc.set_downstream(ComponentId::Bus);
    // An exclusive L1/L2 pair leaves the LLC non-inclusive of the L2s.
    let llc_inclusion = match config.cache.inclusion_policy {
        InclusionPolicy::Exclusive => InclusionPolicy::Nine,
        policy => policy,
    };
    llc.set_upstream_inclusion(llc_inclusion);
    llc
}

/// `hart_count` harts at reset: in machine mode at the start PC, which the
/// riscv-tests and bare-metal images both need.
fn build_harts(config: &Config, hart_count: usize) -> Vec<Hart> {
    let csrs = Csrs {
        mstatus: MSTATUS_DEFAULT_RV64,
        misa: config.misa(),
        stimecmp: u64::MAX,
        vlenb: config.pipeline.vlen.bytes() as u64,
        ..Default::default()
    };
    (0..hart_count)
        .map(|index| {
            Hart::new(HartInit {
                hart_id: HartId::from_index(index),
                regs: RegisterFile::new(config.pipeline.vlen),
                pc: VirtAddr::new(config.general.start_pc),
                csrs: csrs.clone(),
                privilege: PrivilegeMode::Machine,
                pmp: Pmp::new(),
            })
        })
        .collect()
}

/// The stats tree, with every hart's, core's, cache's and the fabric's
/// counters registered.
fn build_stats(
    hart_stat_paths: &[HartPaths],
    topology: &Topology,
    units: &[CoreUnits],
    llc: &Cache,
    coherence: Option<&CoherenceFabric>,
) -> Stats {
    let core_stat_paths: Vec<_> = topology
        .cores
        .iter()
        .zip(units)
        .map(|(c, core)| (core.stat_paths, c.hart_ids[0]))
        .collect();
    let cache_stat_paths: Vec<_> = units
        .iter()
        .flat_map(|core| {
            [core.l1_i_cache.stat_paths, core.l1_d_cache.stat_paths, core.l2_cache.stat_paths]
        })
        .chain(std::iter::once(llc.stat_paths))
        .collect();
    let components: Vec<&dyn StatSource> = cache_stat_paths
        .iter()
        .map(|paths| paths as &dyn StatSource)
        .chain(coherence.map(|fabric| fabric.stat_paths() as &dyn StatSource))
        .collect();
    Stats::for_components(hart_stat_paths, &core_stat_paths, &components)
}

impl SystemState {
    /// The execution view for core `core`: its first hart, its private
    /// micro-architecture, and the uncore.
    ///
    /// # Panics
    ///
    /// Panics if `core` is not a valid core index.
    pub fn core_ctx(&mut self, core: usize) -> CoreCtx<'_> {
        let hart_index = self.uncore.topology.cores[core].hart_ids[0].as_index();
        CoreCtx {
            hart: &mut self.harts[hart_index],
            core: &mut self.cores[core].units,
            uncore: &mut self.uncore,
        }
    }

    /// Core `core`'s pipeline together with the execution view it runs in.
    ///
    /// # Panics
    ///
    /// Panics if `core` is not a valid core index.
    pub fn pipeline_ctx(&mut self, core: usize) -> (&mut PipelineDispatch, CoreCtx<'_>) {
        let hart_index = self.uncore.topology.cores[core].hart_ids[0].as_index();
        let Core { units, pipeline } = &mut self.cores[core];
        let ctx =
            CoreCtx { hart: &mut self.harts[hart_index], core: units, uncore: &mut self.uncore };
        (pipeline, ctx)
    }

    /// Instructions retired across every hart.
    pub fn instructions_retired(&self) -> u64 {
        self.harts.iter().map(|h| h.instructions_retired).sum()
    }

    /// Cycles and instructions retired since the stats were last reset.
    #[must_use]
    pub fn stats_window(&self) -> (u64, u64) {
        let epoch = self.uncore.run.stats_epoch;
        let cycles = self.uncore.reported_cycle() - epoch.cycle;
        (cycles, self.instructions_retired() - epoch.instructions_retired)
    }

    /// Zeroes every stat and starts a new window here.
    pub fn reset_stats(&mut self) {
        self.uncore.kernel.stats.reset();
        self.uncore.run.stats_epoch = StatsEpoch {
            cycle: self.uncore.reported_cycle(),
            instructions_retired: self.instructions_retired(),
        };
    }

    /// Carries out a request the guest made with a pseudo-instruction.
    pub fn apply_sim_op(&mut self, op: SimOp) {
        match op {
            SimOp::ResetStats => self.reset_stats(),
            SimOp::DumpStats { label } => {
                let (cycles, instructions_retired) = self.stats_window();
                let stats = self.uncore.kernel.stats.clone();
                let epoch = self.uncore.run.stats_epoch;
                self.uncore.run.stats_dumps.push(StatsDump {
                    label,
                    epoch,
                    stats,
                    cycles,
                    instructions_retired,
                });
            }
            SimOp::Break { label } => self.uncore.run.pending_break = Some(label),
        }
    }

    #[cfg(test)]
    /// Convenience constructor: allocates a fresh `exit_signal` slot and
    /// builds the system. Use this when the caller doesn't need to share the
    /// signal `Arc` with other components before construction.
    pub fn build(config: &Config) -> Self {
        let config = ValidConfig::try_from(config.clone()).expect("the test config is valid");
        let exit_signal = ExitSignal::new();
        Self::new(&config, None, exit_signal).expect("the test machine builds")
    }

    /// Constructs the system: bus and devices, memory controller, LLC, and
    /// `config.system.hart_count` cores each hosting one hart. `exit_signal`
    /// is cloned into bus-resident devices (`SysCon`, HTIF) so they can
    /// write the harness termination value when triggered.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::RamAllocation`] when the host cannot map the RAM,
    /// and [`SimError::FileRead`] when `disk` names an image it cannot read.
    pub fn new(
        config: &ValidConfig,
        disk: Option<&str>,
        exit_signal: ExitSignal,
    ) -> Result<Self, SimError> {
        let topology = Topology::single_threaded_cores(config.system.hart_count.max(1));
        let hart_count = topology.hart_count();
        let ram_size = config.memory.ram_size;
        let ram = Ram::new(config.system.ram_base.val(), ram_size)
            .map_err(|source| SimError::RamAllocation { bytes: ram_size, source })?;
        let terminal = Terminal::shared(config.system.console);
        let bus = build_bus(config, hart_count, &terminal, &exit_signal, disk)?;
        let mem_controller = build_mem_controller(config);
        let mut l3_cache = build_llc(config, &topology);
        let write_log_line_bytes = match config.cache.l1_d.line_bytes {
            0 => 64,
            line_bytes => line_bytes as u64,
        };
        let harts = build_harts(config, hart_count);

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
        let stats = build_stats(&hart_stat_paths, &topology, &units, &l3_cache, coherence.as_ref());

        let cores = topology
            .cores
            .iter()
            .zip(units)
            .map(|(c, units)| Core {
                units,
                pipeline: PipelineDispatch::new(config, c, VirtAddr::new(config.general.start_pc)),
            })
            .collect();

        Ok(Self {
            harts,
            cores,
            uncore: Uncore {
                trace: TraceControl {
                    armed: config.general.trace_instructions,
                    ..TraceControl::default()
                },
                topology,
                kernel: SimKernel {
                    cycle: 0,
                    event_queue: EventQueue::new(),
                    stats,
                    memory: GlobalMemory::new(Some(ram), hart_count, write_log_line_bytes),
                    config: Config::clone(config),
                },
                bus,
                mem_controller,
                l3_cache,
                coherence,
                per_hart_debug: (0..hart_count).map(|_| HartDebug::default()).collect(),
                run: RunControl::new(exit_signal),
                host: HostSide::new(terminal),
                hart_stat_paths,
            },
        })
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
    use crate::common::PhysAddr;
    use crate::config::Config;

    #[test]
    fn test_cpu_reservation() {
        let config = Config::minimal();
        let mut sys = SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.set_reservation(PhysAddr::new(0x1000));
        assert!(state.check_reservation(PhysAddr::new(0x1000)));
        assert!(state.check_reservation(PhysAddr::new(0x1008)));
        assert!(!state.check_reservation(PhysAddr::new(0x2000)));

        state.clear_reservation();
        assert!(!state.check_reservation(PhysAddr::new(0x1000)));
    }

    #[test]
    fn a_hart_displays_its_pc_and_registers() {
        let config = Config::minimal();
        let state = SystemState::build(&config);
        let text = state.harts[0].to_string();
        assert!(text.starts_with("PC = 0x0000000080000000\n"), "{text}");
        assert_eq!(text.lines().count(), 17, "{text}");
    }

    #[test]
    fn test_cpu_take_exit() {
        let config = Config::minimal();
        let state = SystemState::build(&config);

        assert_eq!(state.uncore.take_exit(), None);
        state.uncore.signal_exit(42);
        assert_eq!(state.uncore.take_exit(), Some(42));
        assert_eq!(state.uncore.take_exit(), None);
    }

    #[test]
    fn every_core_gets_its_own_hart_and_caches() {
        let mut config = Config::minimal();
        config.system.hart_count = 2;
        let sys = SystemState::build(&config);
        assert_eq!(sys.harts.len(), 2);
        assert_eq!(sys.cores.len(), 2);
        assert_eq!(sys.harts[1].hart_id, HartId::new(1));
        assert_eq!(sys.cores[1].units.core_id, crate::common::CoreId::new(1));
        assert_ne!(sys.cores[0].units.l2_cache.id, sys.cores[1].units.l2_cache.id);
        assert_eq!(sys.uncore.l3_cache.id, sys.uncore.topology.llc);
    }
}

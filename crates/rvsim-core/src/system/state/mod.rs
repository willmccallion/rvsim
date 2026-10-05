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
use crate::common::{HartId, PhysAddr};
use crate::config::{Config, InclusionPolicy, MemoryControllerKind};
use crate::isa::privileged::PrivilegeMode;
use crate::sim::components::{ComponentId, MemCtrlId};
use crate::sim::events::EventQueue;
use crate::sim::memory::{GlobalMemory, Ram};
use crate::sim::packet::CacheLevel;
use crate::sim::stats::paths::HartPaths;
use crate::sim::stats::{StatSource, Stats};
use crate::soc::bus::Bus;
use crate::soc::cache::Cache;
use crate::soc::coherence::{self, CoherenceFabric, FabricGeometry};
use crate::soc::devices::{
    Clint, GoldfishRtc, Htif, Plic, SimControl, SimOp, SysCon, Uart, VirtioBlock,
};
use crate::soc::memory::controller::{
    Bandwidth, DramConfig, DramController, MemoryController, SimpleController,
};
use crate::soc::memory::ddr5::Ddr5Controller;
use crate::soc::topology::Topology;
use crate::soc::uncore::debug::HartDebug;
use crate::soc::uncore::{StatsDump, StatsEpoch, TraceControl, Uncore};
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::engine::PipelineDispatch;
use crate::uarch::{Core, CoreUnits};
use std::fs;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

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

impl Deref for SystemState {
    type Target = Uncore;

    fn deref(&self) -> &Uncore {
        &self.uncore
    }
}

impl DerefMut for SystemState {
    fn deref_mut(&mut self) -> &mut Uncore {
        &mut self.uncore
    }
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
        let epoch = self.stats_epoch;
        let cycles = self.reported_cycle() - epoch.cycle;
        (cycles, self.instructions_retired() - epoch.instructions_retired)
    }

    /// Zeroes every stat and starts a new window here.
    pub fn reset_stats(&mut self) {
        self.uncore.stats.reset();
        self.uncore.stats_epoch = StatsEpoch {
            cycle: self.reported_cycle(),
            instructions_retired: self.instructions_retired(),
        };
    }

    /// Carries out a request the guest made through the sim-control device.
    pub fn apply_sim_op(&mut self, op: SimOp) {
        match op {
            SimOp::ResetStats => self.reset_stats(),
            SimOp::DumpStats { label } => {
                let (cycles, instructions_retired) = self.stats_window();
                let stats = self.uncore.stats.clone();
                let epoch = self.uncore.stats_epoch;
                self.uncore.stats_dumps.push(StatsDump {
                    label,
                    epoch,
                    stats,
                    cycles,
                    instructions_retired,
                });
            }
            SimOp::Break { label } => self.uncore.pending_break = Some(label),
        }
    }

    #[cfg(test)]
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
        use crate::isa::csr::{MSTATUS_DEFAULT_RV64, MSTATUS_FS_INIT, MSTATUS_VS_INIT};
        use crate::isa::reg;

        let topology = Topology::single_threaded_cores(config.system.hart_count.max(1));
        let hart_count = topology.hart_count();

        // --- Bus + devices ---------------------------------------------
        let mut bus = Bus::new(config.system.bus_width, config.system.bus_latency, hart_count);

        let ram_base = config.system.ram_base;
        let ram_size = config.memory.ram_size;

        let uart =
            Uart::new(config.system.uart_base, config.system.console, config.system.cpu_clock_mhz);
        let clint = Clint::new(config.system.clint_base, config.system.clint_divider, hart_count);
        let plic = Plic::new(0x0c00_0000, hart_count);

        let mut disk = VirtioBlock::new(config.system.disk_base);
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
            MemoryControllerKind::Dram => Box::new(DramController::new(DramConfig {
                t_cas: config.memory.t_cas,
                t_ras: config.memory.t_ras,
                t_pre: config.memory.t_pre,
                t_rrd: config.memory.t_rrd,
                num_banks: config.memory.num_banks,
                row_size_bytes: config.memory.row_size_bytes,
                t_refi: config.memory.t_refi,
                t_rfc: config.memory.t_rfc,
            })),
            MemoryControllerKind::Simple => {
                let bytes_per_second = config
                    .memory
                    .simple_bandwidth_bytes_per_second()
                    .unwrap_or(std::num::NonZeroU64::MAX);
                Box::new(SimpleController::new(
                    config.memory.simple_latency,
                    Bandwidth::new(bytes_per_second, config.system.cpu_clock_mhz * 1_000_000),
                ))
            }
            MemoryControllerKind::Ddr5 => Box::new(Ddr5Controller::new(
                PhysAddr::new(ram_base),
                ram_size as u64,
                &config.memory.ddr5.to_config(),
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

        bus.attach_ram(MemCtrlId::new(0), ram_base, ram_size as u64);
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

        let csrs = Csrs {
            mstatus,
            misa: configured_misa,
            stimecmp: u64::MAX,
            vlenb: config.pipeline.vlen.bytes() as u64,
            ..Default::default()
        };

        // Direct-mode programs get the bare-metal boot convention: a0 = hart
        // id, a1 = hart count, sp = a stack top every hart shares (a
        // multi-hart runtime carves per-hart stacks below it).
        let fresh_regs = |hart_id: HartId| {
            let mut regs = RegisterFile::new(config.pipeline.vlen);
            if direct_mode {
                let sp = config.general.initial_sp.unwrap_or(config.system.ram_base + 0x100_0000);
                regs.write(reg::REG_SP, sp);
                regs.write(reg::REG_A0, u64::from(hart_id.val()));
                regs.write(reg::REG_A1, hart_count as u64);
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
        let components: Vec<&dyn StatSource> = cache_stat_paths
            .iter()
            .map(|paths| paths as &dyn StatSource)
            .chain(coherence.as_ref().map(|fabric| fabric.stat_paths() as &dyn StatSource))
            .collect();
        let stats = Stats::for_components(&hart_stat_paths, &core_stat_paths, &components);

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
            uncore: Uncore {
                trace: TraceControl {
                    armed: config.general.trace_instructions,
                    ..TraceControl::default()
                },
                topology,
                cycle: 0,
                exited_at: None,
                bus,
                mem_controller,
                l3_cache,
                coherence,
                memory: GlobalMemory::new(
                    Some(Ram::new(ram_base, ram_size)),
                    hart_count,
                    write_log_line_bytes,
                ),
                config: config.clone(),
                per_hart_debug: (0..hart_count).map(|_| HartDebug::default()).collect(),
                panic_detected_at_cycle: None,
                #[cfg(feature = "commit-log")]
                commit_log: None,
                exit_signal,
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
        let mut sys = SystemState::build(&config, "");
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
        let config = Config::default();
        let state = SystemState::build(&config, "");
        let text = state.harts[0].to_string();
        assert!(text.starts_with("PC = 0x0000000080000000\n"), "{text}");
        assert_eq!(text.lines().count(), 17, "{text}");
    }

    #[test]
    fn test_cpu_take_exit() {
        let config = Config::default();
        let state = SystemState::build(&config, "");

        assert_eq!(state.take_exit(), None);
        state.signal_exit(42);
        assert_eq!(state.take_exit(), Some(42));
        assert_eq!(state.take_exit(), None);
    }

    #[test]
    fn direct_mode_harts_boot_with_their_id_and_the_hart_count() {
        use crate::isa::reg;
        let mut config = Config::default();
        config.general.direct_mode = true;
        config.system.hart_count = 3;
        let sys = SystemState::build(&config, "");
        for (index, hart) in sys.harts.iter().enumerate() {
            assert_eq!(hart.regs.read(reg::REG_A0), index as u64);
            assert_eq!(hart.regs.read(reg::REG_A1), 3);
            assert_eq!(hart.regs.read(reg::REG_SP), config.system.ram_base + 0x100_0000);
        }
    }

    #[test]
    fn every_core_gets_its_own_hart_and_caches() {
        let mut config = Config::default();
        config.system.hart_count = 2;
        let sys = SystemState::build(&config, "");
        assert_eq!(sys.harts.len(), 2);
        assert_eq!(sys.cores.len(), 2);
        assert_eq!(sys.harts[1].hart_id, HartId::new(1));
        assert_eq!(sys.cores[1].units.core_id, crate::common::CoreId::new(1));
        assert_ne!(sys.cores[0].units.l2_cache.id, sys.cores[1].units.l2_cache.id);
        assert_eq!(sys.l3_cache.id, sys.topology.llc);
    }
}

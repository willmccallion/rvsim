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

/// Address translation for the pipeline.
pub mod memory;

/// LR/SC reservations shared by all harts.
pub mod reservations;

/// Trap and exception handling logic.
pub mod trap;

use crate::common::{HartId, PhysAddr, RegisterFile};
use crate::config::{Config, MemoryController as MemControllerType};
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::hart::HartInit;
use crate::core::units::cache::Cache;
use crate::core::units::mmu::Mmu;
use crate::core::units::mmu::pmp::Pmp;
use crate::core::{Core, Hart};
use crate::sim::components::{CacheId, ComponentId, MemCtrlId};
use crate::sim::events::EventQueue;
use crate::sim::packet::CacheLevel;
use crate::sim::per_hart_debug::HartDebug;
use crate::sim::stats::Stats;
use crate::sim::topology::Topology;
use crate::soc::devices::{Clint, GoldfishRtc, Htif, Plic, SysCon, Uart, VirtioBlock};
use crate::soc::interconnect::Bus;
use crate::soc::memory::buffer::DramBuffer;
use crate::soc::memory::controller::{
    DramConfig, DramController, MemoryController, SimpleController,
};
use crate::soc::memory::ddr5::Ddr5Controller;
use reservations::ReservationSet;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The uncore: everything shared by all cores.
#[derive(Debug)]
pub struct SharedState {
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
    /// LR/SC reservations, one per hart.
    pub reservations: ReservationSet,
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
    pub core: &'a mut Core,
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
    /// Sets a load reservation for this hart at `addr` (cache-line aligned).
    #[inline]
    pub fn set_reservation(&mut self, addr: PhysAddr) {
        let hart = self.hart.hart_id;
        self.shared.reservations.set(hart, addr);
    }

    /// Returns `true` when this hart holds a reservation covering `addr`.
    #[inline]
    pub fn check_reservation(&self, addr: PhysAddr) -> bool {
        self.shared.reservations.check(self.hart.hart_id, addr)
    }

    /// Clears this hart's load reservation.
    #[inline]
    pub fn clear_reservation(&mut self) {
        let hart = self.hart.hart_id;
        self.shared.reservations.clear(hart);
    }

    /// Breaks every other hart's reservation on the line a store by this
    /// hart writes.
    #[inline]
    pub fn invalidate_other_reservations(&mut self, addr: PhysAddr) {
        let hart = self.hart.hart_id;
        self.shared.reservations.invalidate_others(hart, addr);
    }
}

impl SharedState {
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
            core: &mut self.cores[core],
            shared: &mut self.shared,
        }
    }

    /// Instructions retired across every hart.
    #[must_use]
    pub fn instructions_retired(&self) -> u64 {
        self.harts.iter().map(|h| h.instructions_retired).sum()
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
        use crate::core::arch::csr::{
            MISA_DEFAULT_RV64IMAFDC, MISA_EXT_A, MISA_EXT_C, MISA_EXT_D, MISA_EXT_F, MISA_EXT_I,
            MISA_EXT_M, MISA_EXT_S, MISA_EXT_U, MISA_XLEN_64, MSTATUS_DEFAULT_RV64, MSTATUS_FS,
            MSTATUS_FS_INIT, MSTATUS_MXR, MSTATUS_SIE, MSTATUS_SPIE, MSTATUS_SPP, MSTATUS_SUM,
            MSTATUS_UXL, MSTATUS_VS_INIT,
        };
        use crate::isa::abi;

        let topology = Topology::single_threaded_cores(config.system.hart_count.max(1));
        let hart_count = topology.hart_count();

        // --- Bus + devices ---------------------------------------------
        let mut bus = Bus::new(config.system.bus_width, config.system.bus_latency, hart_count);

        let ram_base = config.system.ram_base;
        let ram_size = config.memory.ram_size;
        let ram_buffer = Arc::new(DramBuffer::new(ram_size));

        let uart = Uart::new(
            config.system.uart_base,
            config.system.uart_to_stderr,
            config.system.uart_quiet,
        );
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
        let rtc = GoldfishRtc::new(0x101000);

        bus.add_device(Box::new(uart));
        bus.add_device(Box::new(disk));
        bus.add_device(Box::new(clint));
        bus.add_device(Box::new(plic));
        bus.add_device(Box::new(syscon));
        bus.add_device(Box::new(rtc));

        if config.system.tohost_addr != 0 {
            let htif = Htif::new(config.system.tohost_addr, exit_signal.clone());
            bus.add_device(Box::new(htif));
        }

        let mem_controller: Box<dyn MemoryController + Send + Sync> = match config.memory.controller {
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
            MemControllerType::Simple => Box::new(SimpleController::new(
                ram_buffer.clone(),
                PhysAddr::new(ram_base),
                config.memory.row_miss_latency,
            )),
            MemControllerType::Ddr5 => Box::new(Ddr5Controller::new(
                ram_buffer.clone(),
                PhysAddr::new(ram_base),
                config.memory.ddr5.to_config(),
                MemCtrlId::new(0),
                config.system.cpu_clock_mhz,
            )),
        };

        let mut l3_cache = Cache::new(topology.llc, CacheLevel::L3, &config.cache.l3);
        l3_cache.set_downstream(ComponentId::Bus);

        let ram_region = crate::soc::memory::RamRegion::new(
            ram_buffer.as_mut_ptr(),
            ram_base,
            ram_size as u64,
        );
        bus.attach_ram(MemCtrlId::new(0), ram_region);

        // --- Hart architectural state ----------------------------------
        let configured_misa = config.pipeline.misa_override.as_ref().map_or_else(
            || {
                MISA_XLEN_64
                    | MISA_EXT_A
                    | MISA_EXT_C
                    | MISA_EXT_D
                    | MISA_EXT_F
                    | MISA_EXT_I
                    | MISA_EXT_M
                    | MISA_EXT_S
                    | MISA_EXT_U
            },
            |override_str| {
                let s = override_str.trim_start_matches("0x");
                u64::from_str_radix(s, 16).unwrap_or(MISA_DEFAULT_RV64IMAFDC)
            },
        );

        let direct_mode = config.general.direct_mode;

        // In direct (SE) mode, enable FP state so user programs can use
        // floating-point instructions without an OS to set mstatus.FS/VS.
        // In full-system mode, firmware/OS is responsible for enabling FP/V.
        let mstatus = if direct_mode {
            MSTATUS_DEFAULT_RV64 | MSTATUS_FS_INIT | MSTATUS_VS_INIT
        } else {
            MSTATUS_DEFAULT_RV64
        };

        // Initialize sstatus as a view of mstatus (spec: sstatus is not a
        // separate register, it's a restricted view of mstatus).
        let sstatus_mask = MSTATUS_SIE
            | MSTATUS_SPIE
            | MSTATUS_SPP
            | MSTATUS_FS
            | MSTATUS_SUM
            | MSTATUS_MXR
            | MSTATUS_UXL;
        let sstatus = mstatus & sstatus_mask;
        let vlenb = config.pipeline.vlen / 8;
        let csrs = Csrs {
            mstatus,
            sstatus,
            misa: configured_misa,
            stimecmp: u64::MAX,
            vlenb: vlenb as u64,
            ..Default::default()
        };

        let fresh_regs = || {
            let mut regs = if direct_mode {
                let sp =
                    config.general.initial_sp.unwrap_or(config.system.ram_base + 0x100_0000);
                let mut r = RegisterFile::new();
                r.write(abi::REG_SP, sp);
                r
            } else {
                RegisterFile::new()
            };
            // Initialize vector register file if VLEN > 0
            if config.pipeline.vlen > 0
                && let Ok(vlen) = crate::core::units::vpu::types::Vlen::new(config.pipeline.vlen)
            {
                regs.init_vpr(vlen);
            }
            regs
        };

        // Always start in Machine mode. The riscv-tests switch to lower modes
        // via their own trap handlers; bare-metal binaries need M-mode too.
        let privilege = PrivilegeMode::Machine;

        let fresh_mmu = || {
            Mmu::new(
                config.memory.tlb_size,
                config.memory.l2_tlb_size,
                config.memory.l2_tlb_ways,
                config.memory.l2_tlb_latency,
                config.memory.software_ad_bits,
                config.memory.paging_mode_max,
            )
        };

        let harts: Vec<Hart> = (0..hart_count)
            .map(|index| {
                let mut hart = Hart::new(HartInit {
                    hart_id: HartId::new(u32::try_from(index).unwrap_or(u32::MAX)),
                    regs: fresh_regs(),
                    pc: config.general.start_pc,
                    csrs: csrs.clone(),
                    privilege,
                    mmu: fresh_mmu(),
                    pmp: Pmp::new(),
                });
                hart.committed_next_pc = config.general.start_pc;
                hart
            })
            .collect();

        let cores: Vec<Core> = topology
            .cores
            .iter()
            .map(|c| Core::new(c.core_id, config, c.l1i.val(), topology.llc))
            .collect();
        for core in &cores {
            l3_cache.add_upstream(ComponentId::Cache(core.l2_cache.id));
        }

        Self {
            harts,
            cores,
            shared: SharedState {
                topology,
                cycle: 0,
                bus,
                mem_controller,
                l3_cache,
                reservations: ReservationSet::new(hart_count),
                config: config.clone(),
                per_hart_debug: (0..hart_count).map(|_| HartDebug::default()).collect(),
                panic_detected_at_cycle: None,
                #[cfg(feature = "commit-log")]
                commit_log: None,
                exit_signal,
                exit_code: None,
                direct_mode,
                event_queue: EventQueue::new(),
                stats: Stats::with_default_registrations(),
            },
        }
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
    fn every_core_gets_its_own_hart_and_caches() {
        let mut config = Config::default();
        config.system.hart_count = 2;
        let sys = SimState::build(&config, "");
        assert_eq!(sys.harts.len(), 2);
        assert_eq!(sys.cores.len(), 2);
        assert_eq!(sys.harts[1].hart_id, HartId::new(1));
        assert_eq!(sys.cores[1].core_id, crate::common::CoreId::new(1));
        assert_ne!(sys.cores[0].l2_cache.id, sys.cores[1].l2_cache.id);
        assert_eq!(sys.l3_cache.id, sys.topology.llc);
    }
}

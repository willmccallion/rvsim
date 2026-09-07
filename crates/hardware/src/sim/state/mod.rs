//! CPU Core Definition and Initialization.
//!
//! Defines the central `SimState` structure holding all architectural processor
//! state. The pipeline lives separately in `Simulator`; this struct owns
//! registers, MMU, caches, and the system bus.

/// Control and Status Register access and management.
pub mod csr;

/// Instruction execution orchestration and pipeline coordination.
pub mod execution;

/// Memory access handling and load/store operations.
pub mod memory;

/// Trap and exception handling logic.
pub mod trap;

use crate::common::{CoreId, HartId, PhysAddr, RegisterFile};
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
use crate::soc::L3_CACHE_ID;
use crate::soc::devices::{Clint, GoldfishRtc, Htif, Plic, SysCon, Uart, VirtioBlock};
use crate::soc::interconnect::Bus;
use crate::soc::memory::buffer::DramBuffer;
use crate::soc::memory::controller::{
    DramConfig, DramController, MemoryController, SimpleController,
};
use crate::stats::SimStats;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// CPU architectural state: registers, caches, MMU, bus, and statistics.
///
/// The pipeline is owned by `Simulator`, not by `SimState`. This struct holds only
/// the architectural state that the pipeline reads and writes.
#[derive(Debug)]
pub struct SimState {
    /// Per-thread architectural state (registers, CSRs, PC, MMU, PMP, ...).
    pub hart: Hart,
    /// Pipeline-private state shared by harts on this core (caches, MSHRs,
    /// branch predictor, prefetch filter, write-combining buffer).
    pub core: Core,

    /// Master clock; every subsystem reads from this.
    pub cycle: u64,
    /// IO interconnect; routes accesses to RAM and MMIO devices.
    pub bus: Bus,
    /// Main memory controller.
    pub mem_controller: MemoryController,
    /// Shared L3 cache (last-level cache; future shared LLC for multi-core).
    pub l3_cache: Cache,

    /// Simulator parameters (cache sizes, ISA capability flags, pipeline
    /// knobs, system layout). Owned by `SimState` transitionally; the bench
    /// view migrates this to `Simulator` later.
    pub config: Config,

    /// Sim-side per-hart debug bookkeeping (hang detection, panic timing,
    /// retire trace). Indexed by `HartId`; transitionally lives here until
    /// the bench view replaces direct `SimState` access.
    pub per_hart_debug: Vec<HartDebug>,

    /// Number of instructions committed (retired). Read on every
    /// INSTRET/MINSTRET CSR access — kept as a dedicated `u64` on the
    /// hot path rather than in the observability tree.
    pub instructions_retired: u64,
    /// Sim-side perf observability counters.
    pub stats: SimStats,
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
    /// Hierarchical statistics tree. Will absorb [`SimStats`] in a later phase.
    pub stats_hier: Stats,
    /// Monotonic counter for assigning fresh [`crate::sim::components::ReqId`]
    /// values to outgoing memory requests.
    pub next_req_id: u64,
}

unsafe impl Send for SimState {}
unsafe impl Sync for SimState {}

impl SimState {
    /// Sets a load reservation at the given address (cache-line aligned).
    #[inline]
    pub(crate) const fn set_reservation(&mut self, addr: PhysAddr) {
        self.hart.set_reservation(addr);
    }

    /// Returns `true` when a reservation covers `addr` (same cache line).
    #[inline]
    pub(crate) const fn check_reservation(&self, addr: PhysAddr) -> bool {
        self.hart.check_reservation(addr)
    }

    /// Clears the load reservation.
    #[inline]
    pub(crate) const fn clear_reservation(&mut self) {
        self.hart.clear_reservation();
    }

    /// Constructs a new CPU with the full system-on-chip (bus, memory
    /// controller, LLC, devices) inlined. `exit_signal` is cloned into
    /// bus-resident devices (`SysCon`, HTIF) so they can write the harness
    /// termination value when triggered.
    pub fn new(config: &Config, disk_path: &str, exit_signal: Arc<AtomicU64>) -> Self {
        use crate::core::arch::csr::{
            MISA_DEFAULT_RV64IMAFDC, MISA_EXT_A, MISA_EXT_C, MISA_EXT_D, MISA_EXT_F, MISA_EXT_I,
            MISA_EXT_M, MISA_EXT_S, MISA_EXT_U, MISA_XLEN_64, MSTATUS_DEFAULT_RV64, MSTATUS_FS,
            MSTATUS_FS_INIT, MSTATUS_MXR, MSTATUS_SIE, MSTATUS_SPIE, MSTATUS_SPP, MSTATUS_SUM,
            MSTATUS_UXL, MSTATUS_VS_INIT,
        };
        use crate::isa::abi;

        // --- Bus + devices ---------------------------------------------
        let mut bus = Bus::new(config.system.bus_width, config.system.bus_latency);

        let ram_base = config.system.ram_base;
        let ram_size = config.memory.ram_size;
        let ram_buffer = Arc::new(DramBuffer::new(ram_size));

        let uart = Uart::new(
            config.system.uart_base,
            config.system.uart_to_stderr,
            config.system.uart_quiet,
        );
        let clint = Clint::new(config.system.clint_base, config.system.clint_divider);
        let plic = Plic::new(0x0c00_0000);

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

        let mem_controller = match config.memory.controller {
            MemControllerType::Dram => MemoryController::Dram(DramController::new(
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
            MemControllerType::Simple => MemoryController::Simple(SimpleController::new(
                ram_buffer.clone(),
                PhysAddr::new(ram_base),
                config.memory.row_miss_latency,
            )),
        };

        let mut l3_cache = Cache::new(L3_CACHE_ID, CacheLevel::L3, &config.cache.l3);
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

        let mut regs = if direct_mode {
            let sp = config.general.initial_sp.unwrap_or(config.system.ram_base + 0x100_0000);
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

        // Always start in Machine mode. The riscv-tests switch to lower modes
        // via their own trap handlers; bare-metal binaries need M-mode too.
        let privilege = PrivilegeMode::Machine;

        let mmu = Mmu::new(
            config.memory.tlb_size,
            config.memory.l2_tlb_size,
            config.memory.l2_tlb_ways,
            config.memory.l2_tlb_latency,
            config.memory.software_ad_bits,
            config.memory.paging_mode_max,
        );

        let mut hart = Hart::new(HartInit {
            hart_id: HartId::new(0),
            regs,
            pc: config.general.start_pc,
            csrs,
            privilege,
            mmu,
            pmp: Pmp::new(),
        });
        hart.committed_next_pc = config.general.start_pc;

        let core = Core::new(CoreId::new(0), config, 0, L3_CACHE_ID);
        l3_cache.add_upstream(ComponentId::Cache(core.l2_cache.id));

        Self {
            hart,
            core,
            cycle: 0,
            bus,
            mem_controller,
            l3_cache,
            config: config.clone(),
            per_hart_debug: vec![HartDebug::default()],
            instructions_retired: 0,
            stats: SimStats::default(),
            #[cfg(feature = "commit-log")]
            commit_log: None,
            exit_signal,
            exit_code: None,
            direct_mode,
            event_queue: EventQueue::new(),
            stats_hier: Stats::new(),
            next_req_id: 0,
        }
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

    /// Convenience constructor: allocates a fresh `exit_signal` slot and
    /// builds the CPU. Use this when the caller doesn't need to share the
    /// signal `Arc` with other components before construction.
    pub fn build(config: &Config, disk_path: &str) -> Self {
        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        Self::new(config, disk_path, exit_signal)
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

    /// Advances all bus-resident devices by one tick and returns this
    /// cycle's interrupt snapshot.
    pub fn bus_tick(&mut self) -> crate::soc::interconnect::BusIrqs {
        self.bus.tick()
    }

    /// Returns the `CacheId` of the shared LLC.
    pub const fn l3_cache_id(&self) -> CacheId {
        self.l3_cache.id
    }

    /// Opens a commit log file for writing retired instruction traces.
    ///
    /// Each retired instruction is logged as `core   0: 0x<pc> (0x<inst>)\n`.
    /// Only available when the `commit-log` Cargo feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::FileRead`] if the file cannot be created.
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

    /// Dumps the current CPU state (PC and registers) to stdout.
    #[inline]
    pub fn dump_state(&self) {
        self.hart.dump_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_cpu_reservation() {
        let config = Config::default();
        let mut state = SimState::build(&config, "");

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
}

//! CPU Core Definition and Initialization.
//!
//! This module defines the central `Cpu` structure, which holds all architectural
//! processor state. The pipeline lives separately in `Simulator`; this struct owns:
//! 1. **State Management:** Registers, program counter, and privilege modes.
//! 2. **Memory Hierarchy:** MMU, TLBs, and multi-level cache simulations.
//! 3. **System Integration:** System bus, devices, and RAM.

/// Control and Status Register access and management.
pub mod csr;

/// Instruction execution orchestration and pipeline coordination.
pub mod execution;

/// Memory access handling and load/store operations.
pub mod memory;

/// Trap and exception handling logic.
pub mod trap;

use crate::common::{PhysAddr, RegisterFile};
use crate::config::{Config, InclusionPolicy};
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::pipeline::write_buffer::WriteCombiningBuffer;
use crate::core::units::bru::BranchPredictorWrapper;
use crate::core::units::cache::CacheSim;
use crate::core::units::cache::mshr::MshrFile;
use crate::core::units::mmu::Mmu;
use crate::core::units::mmu::pmp::Pmp;
use crate::core::units::prefetch::PrefetchFilter;
use crate::soc::System;
use crate::stats::SimStats;

/// CPU architectural state: registers, caches, MMU, bus, and statistics.
///
/// The pipeline is owned by `Simulator`, not by `Cpu`. This struct holds only
/// the architectural state that the pipeline reads and writes.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cpu {
    /// General Purpose and Floating Point Registers.
    pub regs: RegisterFile,
    /// Program Counter.
    pub pc: u64,
    /// Control and Status Registers.
    pub csrs: Csrs,
    /// Current Privilege Mode (M, S, U).
    pub privilege: PrivilegeMode,
    /// Load Reservation address (for LR/SC).
    pub load_reservation: Option<PhysAddr>,

    /// System Bus and Devices.
    pub bus: System,
    /// Memory Management Unit.
    pub mmu: Mmu,
    /// Physical Memory Protection unit.
    pub pmp: Pmp,
    /// L1 Instruction Cache.
    pub l1_i_cache: CacheSim,
    /// L1 Data Cache.
    pub l1_d_cache: CacheSim,
    /// L2 Unified Cache.
    pub l2_cache: CacheSim,
    /// L3 Unified Cache.
    pub l3_cache: CacheSim,
    /// L1D MSHR file for non-blocking cache access (O3 backend only).
    pub l1d_mshrs: MshrFile,
    /// Cache inclusion policy (Inclusive / Exclusive / NINE).
    pub inclusion_policy: InclusionPolicy,
    /// Write Combining Buffer for store coalescing.
    pub wcb: WriteCombiningBuffer,
    /// Shared prefetch filter to deduplicate prefetch requests across cache levels.
    pub prefetch_filter: PrefetchFilter,
    /// Base address of RAM — addresses at or above this go through the
    /// cache hierarchy for latency simulation; addresses below are MMIO.
    pub cache_base: u64,

    /// Branch Predictor Unit.
    pub branch_predictor: BranchPredictorWrapper,
    /// Pipeline width (superscalar degree).
    pub pipeline_width: usize,
    /// True when using an O3 backend with register renaming.
    /// Decode skips intra-bundle RAW hazard checks (rename handles them).
    pub has_register_renaming: bool,
    /// I-cache line size in bytes (for cache-line-aligned fetch).
    pub i_cache_line_bytes: usize,

    /// Enable instruction tracing.
    pub trace: bool,
    /// Exit code if simulation finished.
    pub exit_code: Option<u64>,
    /// Performance statistics.
    pub stats: SimStats,
    /// Direct mode (no translation, flat memory).
    pub direct_mode: bool,
    /// CLINT time divider.
    pub clint_divider: u64,
    /// Last PC (for hang detection).
    pub last_pc: u64,
    /// Hang detection counter.
    pub same_pc_count: u64,
    /// WFI state.
    pub wfi_waiting: bool,
    /// PC when WFI was entered.
    pub wfi_pc: u64,
    /// The PC that the next committed instruction should start at.
    /// Updated after every commit to `entry.pc + entry.inst_size`.
    /// Used as the EPC for interrupts when the ROB is empty, because
    /// `cpu.pc` is the *fetch* PC, which can be far ahead of the commit point.
    pub committed_next_pc: u64,
    /// Raw pointer to the start of simulated RAM.
    ///
    /// # Safety Invariants
    ///
    /// This pointer must maintain the following invariants at all times:
    /// - Points to a valid, allocated memory region of size `(ram_end - ram_start)` bytes
    /// - The memory region remains valid for the entire lifetime of the `Cpu` instance
    /// - All accesses must verify: `ram_start <= address < ram_end` before dereferencing
    /// - The pointer is valid for both reads and writes (memory is mutable)
    /// - Memory is properly aligned for the underlying allocation (even if individual
    ///   accesses use `read_unaligned`/`write_unaligned`)
    /// - No other code may free or reallocate this memory while the CPU exists
    /// - The pointer remains valid across CPU state changes and pipeline operations
    pub ram_ptr: *mut u8,
    /// Physical address where RAM starts.
    pub ram_start: u64,
    /// Physical address where RAM ends (exclusive).
    pub ram_end: u64,

    /// HTIF tohost address range (start, end). Stores in this range bypass the
    /// RAM fast-path and go through the bus so the HTIF device can intercept them.
    pub htif_range: Option<(u64, u64)>,

    /// Ring buffer of (pc, inst) for last N retired instructions (for invalid-PC debug trace).
    pub pc_trace: Vec<(u64, u32)>,
    /// Last invalid PC we printed debug for (avoid duplicate dumps).
    pub last_invalid_pc_debug: Option<u64>,

    /// Set by the backend when a PC redirect occurs (branch misprediction,
    /// trap, FENCE.I, etc.). The pipeline uses this to flush the frontend,
    /// rather than relying solely on `cpu.pc != pc_before` which can miss
    /// redirects when the target happens to equal the current fetch PC.
    pub redirect_pending: bool,

    /// Software-managed A/D bits: PTW faults on A=0 or D=0 instead of
    /// auto-setting them (matches spike's behavior for log comparison).
    pub software_ad_bits: bool,

    /// Trap on misaligned memory accesses instead of handling them natively.
    pub misaligned_access_trap: bool,

    /// Cycle at which a kernel panic was first detected (None if not yet detected).
    /// The simulator runs for 100k more cycles after detection to allow the full
    /// panic message to be printed before exiting.
    pub panic_detected_at_cycle: Option<u64>,

    /// Software-written SEIP bit. SEIP in mip is the OR of this and the PLIC
    /// hardware signal, so we must track the software component separately.
    pub sw_seip: bool,

    /// Optional buffered writer for the commit log (enabled by the `commit-log` feature).
    #[cfg(feature = "commit-log")]
    pub commit_log: Option<std::io::BufWriter<std::fs::File>>,
}

/// Maximum number of (pc, inst) entries kept for invalid-PC debug trace.
pub const PC_TRACE_MAX: usize = 32;

unsafe impl Send for Cpu {}
unsafe impl Sync for Cpu {}

impl Cpu {
    /// Cache line size for reservation granularity (64 bytes)
    const RESERVATION_GRANULE: u64 = 64;

    /// Aligns an address to the reservation granule (cache line boundary)
    #[inline]
    const fn align_reservation_address(addr: PhysAddr) -> PhysAddr {
        PhysAddr(addr.0 & !(Self::RESERVATION_GRANULE - 1))
    }

    /// Sets a load reservation at the given address (cache-line aligned)
    #[inline]
    pub(crate) const fn set_reservation(&mut self, addr: PhysAddr) {
        self.load_reservation = Some(Self::align_reservation_address(addr));
    }

    /// Checks if a reservation exists for the given address
    #[inline]
    pub(crate) const fn check_reservation(&self, addr: PhysAddr) -> bool {
        if let Some(reserved_addr) = self.load_reservation {
            reserved_addr.0 == Self::align_reservation_address(addr).0
        } else {
            false
        }
    }

    /// Clears the load reservation
    #[inline]
    pub(crate) const fn clear_reservation(&mut self) {
        self.load_reservation = None;
    }

    /// Creates a new CPU instance with the specified system and configuration.
    ///
    /// # Arguments
    ///
    /// * `system` - The SOC system containing the bus and devices.
    /// * `config` - The simulator configuration parameters.
    ///
    /// # Returns
    ///
    /// A new `Cpu` instance initialized according to the provided configuration.
    pub fn new(mut system: System, config: &Config) -> Self {
        use crate::core::arch::csr::{
            MISA_DEFAULT_RV64IMAFDC, MISA_EXT_A, MISA_EXT_C, MISA_EXT_D, MISA_EXT_F, MISA_EXT_I,
            MISA_EXT_M, MISA_EXT_S, MISA_EXT_U, MISA_XLEN_64, MSTATUS_DEFAULT_RV64, MSTATUS_FS,
            MSTATUS_FS_INIT, MSTATUS_MXR, MSTATUS_SIE, MSTATUS_SPIE, MSTATUS_SPP, MSTATUS_SUM,
            MSTATUS_UXL,
        };
        use crate::isa::abi;

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
        // floating-point instructions without an OS to set mstatus.FS.
        // In full-system mode, firmware/OS is responsible for enabling FP.
        let mstatus =
            if direct_mode { MSTATUS_DEFAULT_RV64 | MSTATUS_FS_INIT } else { MSTATUS_DEFAULT_RV64 };

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
        let csrs = Csrs {
            mstatus,
            sstatus,
            misa: configured_misa,
            stimecmp: u64::MAX,
            ..Default::default()
        };

        let bp = BranchPredictorWrapper::new(config);

        let (ram_ptr, ram_start, ram_end) =
            system.bus.get_ram_info().unwrap_or((std::ptr::null_mut(), 0, 0));
        let regs = if direct_mode {
            let sp = config.general.initial_sp.unwrap_or(config.system.ram_base + 0x100_0000);
            let mut r = RegisterFile::new();
            r.write(abi::REG_SP, sp);
            r
        } else {
            RegisterFile::new()
        };
        // Always start in Machine mode. The riscv-tests switch to lower modes
        // via their own trap handlers; bare-metal binaries need M-mode too.
        let privilege = PrivilegeMode::Machine;

        Self {
            regs,
            pc: config.general.start_pc,
            trace: config.general.trace_instructions,
            bus: system,
            exit_code: None,
            csrs,
            privilege,
            direct_mode,
            cache_base: config.system.ram_base,
            stats: SimStats::default(),
            branch_predictor: bp,
            l1_i_cache: CacheSim::new(&config.cache.l1_i),
            l1_d_cache: CacheSim::new(&config.cache.l1_d),
            l1d_mshrs: MshrFile::new(config.cache.l1_d.mshr_count, config.cache.l1_d.line_bytes),
            inclusion_policy: config.cache.inclusion_policy,
            wcb: WriteCombiningBuffer::new(config.cache.wcb_entries, config.cache.l1_d.line_bytes),
            prefetch_filter: PrefetchFilter::new(
                if config.cache.l1_d.prefetcher != crate::config::Prefetcher::None
                    || config.cache.l2.prefetcher != crate::config::Prefetcher::None
                {
                    64 // Default filter size when any prefetcher is active
                } else {
                    0 // Disabled when no prefetchers
                },
                config.cache.l1_d.line_bytes,
            ),
            l2_cache: CacheSim::new(&config.cache.l2),
            l3_cache: CacheSim::new(&config.cache.l3),
            mmu: Mmu::new(
                config.memory.tlb_size,
                config.memory.l2_tlb_size,
                config.memory.l2_tlb_ways,
                config.memory.l2_tlb_latency,
                config.memory.software_ad_bits,
            ),
            pmp: Pmp::new(),
            load_reservation: None,
            pipeline_width: config.pipeline.width,
            has_register_renaming: config.pipeline.backend
                == crate::core::pipeline::engine::BackendType::OutOfOrder,
            i_cache_line_bytes: config.cache.l1_i.line_bytes.max(1),
            clint_divider: config.system.clint_divider,
            last_pc: 0,
            same_pc_count: 0,
            wfi_waiting: false,
            wfi_pc: 0,
            committed_next_pc: config.general.start_pc,
            ram_ptr,
            ram_start,
            ram_end,
            htif_range: None,
            pc_trace: Vec::with_capacity(PC_TRACE_MAX),
            last_invalid_pc_debug: None,
            redirect_pending: false,
            software_ad_bits: config.memory.software_ad_bits,
            misaligned_access_trap: config.memory.misaligned_access_trap,
            panic_detected_at_cycle: None,
            sw_seip: false,
            #[cfg(feature = "commit-log")]
            commit_log: None,
        }
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

    /// Retrieves the exit code if the simulation has finished.
    ///
    /// # Returns
    ///
    /// `Some(u64)` containing the exit code if finished, otherwise `None`.
    pub const fn take_exit(&mut self) -> Option<u64> {
        self.exit_code.take()
    }

    /// Dumps the current CPU state (PC and registers) to stdout.
    pub fn dump_state(&self) {
        println!("PC = {:#018x}", self.pc);
        self.regs.dump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::soc::builder::System;

    #[test]
    fn test_cpu_reservation() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.set_reservation(PhysAddr::new(0x1000));
        assert!(cpu.check_reservation(PhysAddr::new(0x1000)));
        assert!(cpu.check_reservation(PhysAddr::new(0x1008))); // same cache line
        assert!(!cpu.check_reservation(PhysAddr::new(0x2000))); // different cache line

        cpu.clear_reservation();
        assert!(!cpu.check_reservation(PhysAddr::new(0x1000)));
    }

    #[test]
    fn test_cpu_dump_state_no_panic() {
        let config = Config::default();
        let system = System::new(&config, "");
        let cpu = Cpu::new(system, &config);
        cpu.dump_state();
    }

    #[test]
    fn test_cpu_take_exit() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        assert_eq!(cpu.take_exit(), None);
        cpu.exit_code = Some(42);
        assert_eq!(cpu.take_exit(), Some(42));
        assert_eq!(cpu.take_exit(), None);
    }
}

//! CPU Core Definition and Initialization.
//!
//! This module defines the central `Cpu` structure, which serves as the container for the
//! entire processor state. It coordinates the following:
//! 1. **State Management:** Maintains registers, program counter, and privilege modes.
//! 2. **Pipeline Control:** Manages latches and shadow buffers for five-stage execution.
//! 3. **Memory Hierarchy:** Integrates MMU, TLBs, and multi-level cache simulations.
//! 4. **System Integration:** Interfaces with the system bus, devices, and RAM.

/// Control and Status Register access and management.
pub mod csr;

/// Instruction execution orchestration and pipeline coordination.
pub mod execution;

/// Memory access handling and load/store operations.
pub mod memory;

/// Trap and exception handling logic.
pub mod trap;

use crate::common::RegisterFile;
use crate::config::Config;
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::pipeline::latches::{
    ExMem, ExMemEntry, IdEx, IdExEntry, IfId, IfIdEntry, MemWb, MemWbEntry,
};
use crate::core::units::bru::BranchPredictorWrapper;
use crate::core::units::cache::CacheSim;
use crate::core::units::mmu::Mmu;
use crate::soc::System;
use crate::stats::SimStats;

/// Main CPU structure containing all processor state and components.
///
/// The CPU orchestrates instruction execution through the five-stage pipeline,
/// manages memory hierarchy, handles traps, and tracks performance statistics.
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
    pub load_reservation: Option<u64>,

    /// System Bus and Devices.
    pub bus: System,
    /// Memory Management Unit.
    pub mmu: Mmu,
    /// L1 Instruction Cache.
    pub l1_i_cache: CacheSim,
    /// L1 Data Cache.
    pub l1_d_cache: CacheSim,
    /// L2 Unified Cache.
    pub l2_cache: CacheSim,
    /// L3 Unified Cache.
    pub l3_cache: CacheSim,
    /// Base address for MMIO (used to bypass cache).
    pub mmio_base: u64,

    /// IF/ID Latch.
    pub if_id: IfId,
    /// ID/EX Latch.
    pub id_ex: IdEx,
    /// EX/MEM Latch.
    pub ex_mem: ExMem,
    /// MEM/WB Latch.
    pub mem_wb: MemWb,
    /// Writeback Latch (for forwarding).
    pub wb_latch: MemWb,
    /// Branch Predictor Unit.
    pub branch_predictor: BranchPredictorWrapper,
    /// Pipeline width (superscalar degree).
    pub pipeline_width: usize,

    /// Enable instruction tracing.
    pub trace: bool,
    /// Exit code if simulation finished.
    pub exit_code: Option<u64>,
    /// Performance statistics.
    pub stats: SimStats,
    /// Direct mode (no translation, flat memory).
    pub direct_mode: bool,
    /// Stall counter.
    pub stall_cycles: u64,
    /// ALU operation timer (for multi-cycle ops).
    pub alu_timer: u64,
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
    /// Interrupt inhibit flag (for one cycle after CSR write).
    pub interrupt_inhibit_one_cycle: bool,

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

    /// Shadow buffer for IF/ID pipeline latch to avoid allocation churn.
    pub if_id_shadow: Vec<IfIdEntry>,
    /// Shadow buffer for ID/EX pipeline latch.
    pub id_ex_shadow: Vec<IdExEntry>,
    /// Shadow buffer for EX/MEM pipeline latch.
    pub ex_mem_shadow: Vec<ExMemEntry>,
    /// Shadow buffer for MEM/WB pipeline latch.
    pub mem_wb_shadow: Vec<MemWbEntry>,

    /// Ring buffer of (pc, inst) for last N retired instructions (for invalid-PC debug trace).
    pub pc_trace: Vec<(u64, u32)>,
    /// Last invalid PC we printed debug for (avoid duplicate dumps).
    pub last_invalid_pc_debug: Option<u64>,
}

/// Maximum number of (pc, inst) entries kept for invalid-PC debug trace.
pub const PC_TRACE_MAX: usize = 32;

unsafe impl Send for Cpu {}
unsafe impl Sync for Cpu {}

impl Cpu {
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
            MISA_EXT_M, MISA_EXT_S, MISA_EXT_U, MISA_XLEN_64, MSTATUS_DEFAULT_RV64,
        };
        use crate::isa::abi;

        let configured_misa = if let Some(ref override_str) = config.pipeline.misa_override {
            let s = override_str.trim_start_matches("0x");
            u64::from_str_radix(s, 16).unwrap_or(MISA_DEFAULT_RV64IMAFDC)
        } else {
            let mut val = MISA_XLEN_64;
            val |= MISA_EXT_A;
            val |= MISA_EXT_C;
            val |= MISA_EXT_D;
            val |= MISA_EXT_F;
            val |= MISA_EXT_I;
            val |= MISA_EXT_M;
            val |= MISA_EXT_S;
            val |= MISA_EXT_U;
            val
        };

        let csrs = Csrs {
            mstatus: MSTATUS_DEFAULT_RV64,
            misa: configured_misa,
            ..Default::default()
        };

        let bp = BranchPredictorWrapper::new(config);

        let (ram_ptr, ram_start, ram_end) =
            system
                .bus
                .get_ram_info()
                .unwrap_or((std::ptr::null_mut(), 0, 0));

        let direct_mode = config.general.direct_mode;
        let (privilege, regs) = if direct_mode {
            let sp = config
                .general
                .initial_sp
                .unwrap_or(config.system.ram_base + 0x100_0000);
            let mut r = RegisterFile::new();
            r.write(abi::REG_SP, sp);
            (PrivilegeMode::User, r)
        } else {
            (PrivilegeMode::Machine, RegisterFile::new())
        };

        Self {
            regs,
            pc: config.general.start_pc,
            trace: config.general.trace_instructions,
            bus: system,
            exit_code: None,
            csrs,
            privilege,
            direct_mode,
            mmio_base: config.system.ram_base,
            if_id: IfId::default(),
            id_ex: IdEx::default(),
            ex_mem: ExMem::default(),
            mem_wb: MemWb::default(),
            wb_latch: MemWb::default(),
            stats: SimStats::default(),
            branch_predictor: bp,
            l1_i_cache: CacheSim::new(&config.cache.l1_i),
            l1_d_cache: CacheSim::new(&config.cache.l1_d),
            l2_cache: CacheSim::new(&config.cache.l2),
            l3_cache: CacheSim::new(&config.cache.l3),
            stall_cycles: 0,
            alu_timer: 0,
            mmu: Mmu::new(config.memory.tlb_size),
            load_reservation: None,
            pipeline_width: config.pipeline.width,
            clint_divider: config.system.clint_divider,
            last_pc: 0,
            same_pc_count: 0,
            wfi_waiting: false,
            wfi_pc: 0,
            interrupt_inhibit_one_cycle: false,
            ram_ptr,
            ram_start,
            ram_end,
            if_id_shadow: Vec::with_capacity(config.pipeline.width),
            id_ex_shadow: Vec::with_capacity(config.pipeline.width),
            ex_mem_shadow: Vec::with_capacity(config.pipeline.width),
            mem_wb_shadow: Vec::with_capacity(config.pipeline.width),
            pc_trace: Vec::with_capacity(PC_TRACE_MAX),
            last_invalid_pc_debug: None,
        }
    }

    /// Retrieves the exit code if the simulation has finished.
    ///
    /// # Returns
    ///
    /// `Some(u64)` containing the exit code if finished, otherwise `None`.
    pub fn take_exit(&mut self) -> Option<u64> {
        self.exit_code.take()
    }

    /// Dumps the current CPU state (PC and registers) to stdout.
    pub fn dump_state(&self) {
        println!("PC = {:#018x}", self.pc);
        self.regs.dump();
    }
}

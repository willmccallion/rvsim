pub mod branch_predictor;
pub mod cache;
pub mod control;
pub mod pipeline;
pub mod stages;

use self::branch_predictor::BranchPredictor;
use self::cache::CacheSim;
use crate::devices::Bus;
use crate::register_file::RegisterFile;
use crate::stats::SimStats;
use pipeline::{EXMEM, IDEx, IFID, MEMWB};
use stages::{decode_stage, execute_stage, fetch_stage, mem_stage, wb_stage};

pub const L2_LATENCY: u64 = 10;
pub const L3_LATENCY: u64 = 40;
pub const RAM_LATENCY: u64 = 150;

/// Debug-only println
#[macro_export]
macro_rules! dbg_println {
    () => { #[cfg(debug_assertions)] { eprintln!(); } };
    ($($arg:tt)*) => { #[cfg(debug_assertions)] { eprintln!($($arg)*); } };
}

#[derive(Default)]
struct Csrs {
    mstatus: u64,
    sstatus: u64,
    mepc: u64,
    sepc: u64,
    mtvec: u64,
    stvec: u64,
    scause: u64,
    sscratch: u64,
    satp: u64,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum AccessType {
    Read,
    Write,
    Execute,
}

pub struct Cpu {
    pub regs: RegisterFile,
    pub pc: u64,
    pub trace: bool,
    pub bus: Bus,
    pub exit_code: Option<u64>,

    csrs: Csrs,
    pub privilege: u8, // 0=User, 1=Supervisor, 3=Machine

    pub if_id: IFID,
    pub id_ex: IDEx,
    pub ex_mem: EXMEM,
    pub mem_wb: MEMWB,
    pub wb_latch: MEMWB,

    pub stats: SimStats,
    pub branch_predictor: BranchPredictor,

    pub l1_i_cache: CacheSim,
    pub l1_d_cache: CacheSim,
    pub l2_cache: CacheSim,
    pub l3_cache: CacheSim,

    pub stall_cycles: u64,
    pub alu_timer: u64,
}

impl Cpu {
    pub fn new(reset_pc: u64, trace: bool, bus: Bus) -> Self {
        Self {
            regs: RegisterFile::new(),
            pc: reset_pc,
            trace,
            bus,
            exit_code: None,
            csrs: Csrs::default(),
            privilege: 3, // Machine
            if_id: IFID::default(),
            id_ex: IDEx::default(),
            ex_mem: EXMEM::default(),
            mem_wb: MEMWB::default(),
            wb_latch: MEMWB::default(),

            stats: SimStats::default(),
            branch_predictor: BranchPredictor::new(),

            // L1: 16KB, 64B lines, 4-way
            l1_i_cache: CacheSim::new(16 * 1024, 64, 4),
            l1_d_cache: CacheSim::new(16 * 1024, 64, 4),

            // L2: 128KB, 64B lines, 8-way (Unified)
            l2_cache: CacheSim::new(128 * 1024, 64, 8),

            // L3: 2MB, 64B lines, 16-way (Unified)
            l3_cache: CacheSim::new(2 * 1024 * 1024, 64, 16),

            stall_cycles: 0,
            alu_timer: 0,
        }
    }

    /// Simulates MMU translation.
    pub fn translate(&mut self, vaddr: u64, _access: AccessType) -> (u64, u64, Option<String>) {
        let satp = self.csrs.satp;
        let mode = (satp >> 60) & 0xF;

        if self.privilege == 3 || mode == 0 {
            return (vaddr, 0, None);
        }

        let tlb_hit = (self.stats.cycles % 100) != 0;
        if tlb_hit {
            (vaddr, 0, None)
        } else {
            (vaddr, 20, None)
        }
    }

    /// Simulates memory access through the hierarchy.
    pub fn simulate_memory_access(&mut self, addr: u64, is_inst: bool, is_write: bool) -> u64 {
        let mut total_penalty = 0;

        let (l1_hit, l1_wb_penalty) = if is_inst {
            self.l1_i_cache.access(addr, false, L2_LATENCY)
        } else {
            self.l1_d_cache.access(addr, is_write, L2_LATENCY)
        };

        total_penalty += l1_wb_penalty;

        if l1_hit {
            if is_inst {
                self.stats.icache_hits += 1;
            } else {
                self.stats.dcache_hits += 1;
            }
            return total_penalty;
        }

        if is_inst {
            self.stats.icache_misses += 1;
        } else {
            self.stats.dcache_misses += 1;
        }

        total_penalty += L2_LATENCY;
        let (l2_hit, l2_wb_penalty) = self.l2_cache.access(addr, is_write, L3_LATENCY);
        total_penalty += l2_wb_penalty;

        if l2_hit {
            self.stats.l2_hits += 1;
            return total_penalty;
        }
        self.stats.l2_misses += 1;

        total_penalty += L3_LATENCY;
        let (l3_hit, l3_wb_penalty) = self.l3_cache.access(addr, is_write, RAM_LATENCY);
        total_penalty += l3_wb_penalty;

        if l3_hit {
            self.stats.l3_hits += 1;
            return total_penalty;
        }
        self.stats.l3_misses += 1;

        total_penalty += RAM_LATENCY;
        total_penalty
    }

    /// Handles a Trap
    pub fn trap(&mut self, cause: u64, epc: u64) {
        if self.trace {
            eprintln!(">> TRAP: Cause={} EPC={:#x}", cause, epc);
        }

        self.csrs.sepc = epc;
        self.csrs.scause = cause;

        let mut sstatus = self.csrs.sstatus;
        if self.privilege == 0 {
            sstatus &= !(1 << 8);
        } else {
            sstatus |= 1 << 8;
        }
        self.csrs.sstatus = sstatus;

        let vector = self.csrs.stvec & !3;
        self.pc = vector;

        self.privilege = 1;

        self.if_id = Default::default();
        self.id_ex = Default::default();
        self.ex_mem = Default::default();
    }

    pub fn tick(&mut self) -> Result<(), String> {
        if self.trace {
            self.print_pipeline_diagram();
        }

        if self.stall_cycles > 0 {
            self.stall_cycles -= 1;
            self.stats.cycles += 1;
            self.stats.stalls_mem += 1;
            self.track_mode_cycles();
            return Ok(());
        }

        if self.alu_timer > 0 {
            self.alu_timer -= 1;
            self.stats.cycles += 1;
            self.track_mode_cycles();
            return Ok(());
        }

        self.stats.cycles += 1;
        self.track_mode_cycles();

        wb_stage(self)?;

        if self.exit_code.is_some() {
            return Ok(());
        }

        self.wb_latch = self.mem_wb.clone();

        mem_stage(self)?;
        execute_stage(self)?;

        let is_load_use_hazard =
            crate::cpu::control::need_stall_load_use(&self.id_ex, self.if_id.inst);

        if is_load_use_hazard {
            self.id_ex = IDEx::default();
            self.stats.stalls_data += 1;
        } else {
            decode_stage(self)?;
            fetch_stage(self)?;
        }
        self.regs.write(0, 0);
        Ok(())
    }

    fn track_mode_cycles(&mut self) {
        match self.privilege {
            0 => self.stats.cycles_user += 1,
            1 => self.stats.cycles_kernel += 1,
            3 => self.stats.cycles_machine += 1,
            _ => {}
        }
    }

    pub fn print_stats(&self) {
        self.stats.print();
    }

    pub fn take_exit(&mut self) -> Option<u64> {
        self.exit_code.take()
    }

    pub fn dump_state(&self) {
        println!("PC = {:#018x}", self.pc);
        let r = self.regs.dump();
        for i in (0..32).step_by(2) {
            println!(
                "x{:<2} = {:#018x}    x{:<2} = {:#018x}",
                i,
                r[i],
                i + 1,
                r[i + 1]
            );
        }
    }

    #[inline]
    pub fn read_inst(&mut self, pc: u64) -> u32 {
        self.bus.read_u32(pc)
    }

    #[inline]
    pub fn load_u8(&mut self, a: u64) -> u8 {
        self.bus.read_u8(a)
    }

    #[inline]
    pub fn load_u16(&mut self, a: u64) -> u16 {
        self.bus.read_u16(a)
    }

    #[inline]
    pub fn load_u32(&mut self, a: u64) -> u32 {
        self.bus.read_u32(a)
    }

    #[inline]
    pub fn load_u64(&mut self, a: u64) -> u64 {
        self.bus.read_u64(a)
    }

    #[inline]
    pub fn store_u8(&mut self, a: u64, v: u8) {
        self.bus.write_u8(a, v)
    }

    #[inline]
    pub fn store_u16(&mut self, a: u64, v: u16) {
        self.bus.write_u16(a, v)
    }

    #[inline]
    pub fn store_u32(&mut self, a: u64, v: u32) {
        self.bus.write_u32(a, v)
    }

    #[inline]
    pub fn store_u64(&mut self, a: u64, v: u64) {
        self.bus.write_u64(a, v)
    }

    pub(crate) fn csr_read(&self, addr: u32) -> u64 {
        match addr {
            0x300 => self.csrs.mstatus,
            0x100 => self.csrs.sstatus,
            0x341 => self.csrs.mepc,
            0x141 => self.csrs.sepc,
            0x305 => self.csrs.mtvec,
            0x105 => self.csrs.stvec,
            0x142 => self.csrs.scause,
            0x140 => self.csrs.sscratch,
            0x180 => self.csrs.satp,
            _ => 0,
        }
    }

    pub(crate) fn csr_write(&mut self, addr: u32, val: u64) {
        match addr {
            0x300 => self.csrs.mstatus = val,
            0x100 => self.csrs.sstatus = val,
            0x341 => {
                self.csrs.mepc = val & !1;
                if self.trace {
                    eprintln!("CSR write: mepc <- {:#x}", self.csrs.mepc);
                }
            }
            0x141 => {
                self.csrs.sepc = val & !1;
                if self.trace {
                    eprintln!("CSR write: sepc <- {:#x}", self.csrs.sepc);
                }
            }
            0x305 => self.csrs.mtvec = val,
            0x105 => self.csrs.stvec = val,
            0x142 => self.csrs.scause = val,
            0x140 => self.csrs.sscratch = val,
            0x180 => self.csrs.satp = val,
            _ => {}
        }
    }

    pub fn print_pipeline_diagram(&self) {
        if !self.trace {
            return;
        }

        let fmt_stage = |pc: u64, inst: u32, _label: &str| -> String {
            if inst == 0x13 || inst == 0 {
                format!("[{:^8}]", "nop")
            } else {
                format!("[{:08x}]", pc)
            }
        };

        eprintln!(
            "{} -> {} -> {} -> {} -> {}",
            fmt_stage(self.if_id.pc, self.if_id.inst, "IF"),
            fmt_stage(self.id_ex.pc, self.id_ex.inst, "ID"),
            fmt_stage(self.ex_mem.pc, self.ex_mem.inst, "EX"),
            fmt_stage(self.mem_wb.pc, self.mem_wb.inst, "MEM"),
            fmt_stage(self.wb_latch.pc, self.wb_latch.inst, "WB"),
        );
    }

    pub(crate) fn do_mret(&mut self) {
        let target = self.csrs.mepc & !1;
        assert!(target != 0, "MRET with mepc=0");
        self.pc = target;
        self.privilege = 1; // Return to Supervisor
        self.if_id = IFID::default();
        self.id_ex = IDEx::default();
        if self.trace {
            eprintln!("MRET -> pc={:#x}", self.pc);
        }
    }

    pub(crate) fn do_sret(&mut self) {
        let target = self.csrs.sepc & !1;
        assert!(target != 0, "SRET with sepc=0");
        self.pc = target;

        let spp = (self.csrs.sstatus >> 8) & 1;
        self.privilege = spp as u8;

        self.if_id = IFID::default();
        self.id_ex = IDEx::default();
        if self.trace {
            eprintln!("SRET -> pc={:#x} priv={}", self.pc, self.privilege);
        }
    }
}

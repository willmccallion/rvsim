//! Main Execution Loop.
//!
//! This module implements the core execution cycle of the CPU. It performs the following:
//! 1. **Pipeline Coordination:** Orchestrates the movement of instructions through the five-stage pipeline.
//! 2. **Interrupt Handling:** Monitors and processes timer, external, and software interrupts.
//! 3. **Timing Management:** Updates simulation cycles and handles multi-cycle operation stalls.
//! 4. **Observability:** Provides tracing and pipeline visualization for debugging.

use super::Cpu;
use crate::common::constants::{
    DEBUG_PC_END, DEBUG_PC_START, HANG_DETECTION_THRESHOLD, PAGE_OFFSET_MASK, PAGE_SHIFT,
    STATUS_UPDATE_INTERVAL, VPN_MASK, WFI_INSTRUCTION,
};
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::pipeline::{
    hazards,
    stages::{decode_stage, execute_stage, fetch_stage, mem_stage, wb_stage},
};
use crate::isa::abi;

impl Cpu {
    /// Advances the CPU state by one clock cycle.
    ///
    /// This function executes all pipeline stages, handles pending interrupts, updates
    /// timers, and manages stall cycles.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success or an error string on failure.
    pub fn tick(&mut self) -> Result<(), String> {
        if let Some(code) = self.bus.check_exit() {
            self.exit_code = Some(code);
            return Ok(());
        }

        if self.bus.check_kernel_panic() {
            eprintln!("\n[!] Kernel panic detected - exiting simulator");
            self.exit_code = Some(1);
            return Ok(());
        }

        #[allow(clippy::absurd_extreme_comparisons)]
        if self.pc >= DEBUG_PC_START && self.pc <= DEBUG_PC_END {
            self.trace = true;
        }

        if self.pc == self.last_pc {
            self.same_pc_count += 1;
            if self.same_pc_count == HANG_DETECTION_THRESHOLD {
                let inst = if let Some((ppn, _, _, _, _)) =
                    self.mmu.dtlb.lookup((self.pc >> PAGE_SHIFT) & VPN_MASK)
                {
                    let paddr = (ppn << PAGE_SHIFT) | (self.pc & PAGE_OFFSET_MASK);
                    self.bus.bus.read_u32(paddr)
                } else {
                    0
                };

                if self.trace {
                    if inst == WFI_INSTRUCTION {
                        println!(
                            "\n[CPU] Stuck in WFI at {:#x}. Waiting for interrupt...",
                            self.pc
                        );
                    } else {
                        println!(
                            "\n[CPU] POTENTIAL HANG: Stuck at PC {:#x} (Inst: {:#010x})",
                            self.pc, inst
                        );
                    }
                }
            }
        } else {
            self.last_pc = self.pc;
            self.same_pc_count = 0;
        }

        let prev_priv = self.privilege;

        let (timer_irq, meip, seip) = self.bus.tick();

        let mut mip = self.csrs.mip;

        if timer_irq {
            mip |= csr::MIP_MTIP;
        } else {
            mip &= !csr::MIP_MTIP;
        }

        if meip {
            mip |= csr::MIP_MEIP;
        } else {
            mip &= !csr::MIP_MEIP;
        }
        if seip {
            mip |= csr::MIP_SEIP;
        } else {
            mip &= !csr::MIP_SEIP;
        }

        let mtime = self.stats.cycles / self.clint_divider;
        if self.csrs.stimecmp > 0 {
            if mtime >= self.csrs.stimecmp {
                mip |= csr::MIP_STIP;
            } else {
                mip &= !csr::MIP_STIP;
            }
        }

        self.csrs.mip = mip;

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

        wb_stage(self);
        if self.exit_code.is_some() {
            return Ok(());
        }

        self.wb_latch = self.mem_wb.clone();
        mem_stage(self);

        if !self.wfi_waiting {
            let is_load_use_hazard = hazards::need_stall_load_use(&self.id_ex, &self.if_id);

            execute_stage(self);

            if is_load_use_hazard {
                self.stats.stalls_data += 1;
            } else {
                decode_stage(self);

                if self.if_id.entries.is_empty() {
                    fetch_stage(self);
                }
            }
        }

        self.regs.write(abi::REG_ZERO, 0);

        if self.trace {
            if self.privilege != prev_priv {
                println!(
                    "[CPU] Mode Switch: {} -> {} (PC={:#x})",
                    prev_priv.name(),
                    self.privilege.name(),
                    self.pc
                );
            }

            if self.stats.cycles % STATUS_UPDATE_INTERVAL == 0 {
                let mode_name = match self.privilege {
                    PrivilegeMode::Machine => "M",
                    PrivilegeMode::Supervisor => "S",
                    PrivilegeMode::User => "U",
                };
                println!(
                    "[Status] Cycles: {:>10} | PC: {:#010x} | Mode: {}",
                    self.stats.cycles, self.pc, mode_name
                );
            }
        }

        Ok(())
    }

    /// Tracks cycles spent in each privilege mode for statistics.
    fn track_mode_cycles(&mut self) {
        match self.privilege {
            PrivilegeMode::User => self.stats.cycles_user += 1,
            PrivilegeMode::Supervisor => self.stats.cycles_kernel += 1,
            PrivilegeMode::Machine => self.stats.cycles_machine += 1,
        }
    }

    /// Prints a diagram of the current pipeline state.
    pub fn print_pipeline_diagram(&self) {
        eprintln!(
            "IF:{} -> ID:{} -> EX:{} -> MEM:{} -> WB:{}",
            self.if_id.entries.len(),
            self.id_ex.entries.len(),
            self.ex_mem.entries.len(),
            self.mem_wb.entries.len(),
            self.wb_latch.entries.len()
        );
    }
}

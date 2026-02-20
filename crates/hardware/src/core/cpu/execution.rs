//! Main Execution Loop.
//!
//! This module implements the core execution cycle of the CPU. It performs the following:
//! 1. **Pipeline Coordination:** Orchestrates the movement of instructions through the pipeline.
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
use crate::isa::abi;

impl Cpu {
    /// Pre-tick: exit checks, interrupts, timers, cycle counting.
    ///
    /// Returns `Ok(true)` if the pipeline should be skipped this cycle
    /// (e.g. due to ALU timer stall or exit), `Ok(false)` to run the pipeline.
    pub fn pre_tick(&mut self) -> Result<bool, String> {
        if let Some(code) = self.bus.check_exit() {
            self.exit_code = Some(code);
            return Ok(true);
        }

        if self.bus.check_kernel_panic() {
            eprintln!("\n[!] Kernel panic detected - exiting simulator");
            self.exit_code = Some(1);
            return Ok(true);
        }

        #[allow(clippy::absurd_extreme_comparisons)]
        if self.pc >= DEBUG_PC_START && self.pc <= DEBUG_PC_END {
            self.trace = true;
        }

        if self.pc == self.last_pc {
            self.same_pc_count += 1;
            if self.same_pc_count == HANG_DETECTION_THRESHOLD {
                let inst = if let Some((ppn, _, _, _, _, _)) =
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

        self.stats.cycles += 1;
        self.track_mode_cycles();

        Ok(false)
    }

    /// Post-tick: zero x0, privilege tracing, status printing.
    pub fn post_tick(&mut self, prev_priv: PrivilegeMode) {
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

            if self.stats.cycles.is_multiple_of(STATUS_UPDATE_INTERVAL) {
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
        eprintln!("[Pipeline] 10-stage");
    }
}

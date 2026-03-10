//! Main Execution Loop.
//!
//! This module implements the core execution cycle of the CPU. It performs the following:
//! 1. **Pipeline Coordination:** Orchestrates the movement of instructions through the pipeline.
//! 2. **Interrupt Handling:** Monitors and processes timer, external, and software interrupts.
//! 3. **Timing Management:** Updates simulation cycles and handles multi-cycle operation stalls.
//! 4. **Observability:** Provides tracing and pipeline visualization for debugging.

use super::Cpu;
use crate::common::constants::{
    HANG_DETECTION_THRESHOLD, PAGE_OFFSET_MASK, PAGE_SHIFT, STATUS_UPDATE_INTERVAL, VPN_MASK,
    WFI_INSTRUCTION,
};
use crate::common::{Asid, SimError, Vpn};
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::isa::abi;
use crate::trace_trap;

impl Cpu {
    /// Pre-tick: exit checks, interrupts, timers, cycle counting.
    ///
    /// Returns `Ok(true)` if the pipeline should be skipped this cycle
    /// (e.g. due to ALU timer stall or exit), `Ok(false)` to run the pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::KernelPanic`] when the bus panic sentinel fires.
    pub fn pre_tick(&mut self) -> Result<bool, SimError> {
        if let Some(code) = self.bus.check_exit() {
            self.exit_code = Some(code);
            return Ok(true);
        }

        if self.bus.check_kernel_panic() {
            let detected_at = *self.panic_detected_at_cycle.get_or_insert(self.stats.cycles);
            if self.stats.cycles.saturating_sub(detected_at) >= 10_000 {
                return Err(SimError::KernelPanic { cycle: detected_at });
            }
        }

        if self.pc == self.last_pc {
            self.same_pc_count += 1;
            if self.same_pc_count == HANG_DETECTION_THRESHOLD {
                let asid = Asid::new(
                    ((self.csrs.satp >> csr::SATP_ASID_SHIFT) & csr::SATP_ASID_MASK) as u16,
                );
                let inst = if let Some(hit) =
                    self.mmu.dtlb.lookup(Vpn::new((self.pc >> PAGE_SHIFT) & VPN_MASK), asid)
                {
                    let paddr = crate::common::PhysAddr::new(
                        hit.ppn.to_addr() | (self.pc & PAGE_OFFSET_MASK),
                    );
                    self.bus.bus.read_u32(paddr)
                } else {
                    let paddr = crate::common::PhysAddr::new(self.pc);
                    if self.bus.bus.is_valid_address(paddr) {
                        // No TLB entry — likely M-mode with paging disabled;
                        // treat PC as a physical address.
                        self.bus.bus.read_u32(paddr)
                    } else {
                        0
                    }
                };

                if inst == WFI_INSTRUCTION {
                    trace_trap!(self.trace;
                        event = "wfi-wait",
                        pc    = %crate::trace::Hex(self.pc),
                        "CPU stuck in WFI — waiting for interrupt"
                    );
                } else {
                    trace_trap!(self.trace;
                        event = "potential-hang",
                        pc    = %crate::trace::Hex(self.pc),
                        inst  = inst,
                        "CPU potential hang detected"
                    );
                }
            }
        } else {
            self.last_pc = self.pc;
            self.same_pc_count = 0;
        }

        let (timer_irq, msip, meip, seip) = self.bus.tick();

        let mut mip = self.csrs.mip;

        if timer_irq {
            mip |= csr::MIP_MTIP;
        } else {
            mip &= !csr::MIP_MTIP;
        }

        if msip {
            mip |= csr::MIP_MSIP;
        } else {
            mip &= !csr::MIP_MSIP;
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
        if mtime >= self.csrs.stimecmp {
            mip |= csr::MIP_STIP;
        } else {
            mip &= !csr::MIP_STIP;
        }

        self.csrs.mip = mip;

        self.stats.cycles += 1;
        self.track_mode_cycles();

        Ok(false)
    }

    /// Post-tick: zero x0, privilege tracing, status printing.
    pub fn post_tick(&mut self, prev_priv: PrivilegeMode) {
        self.regs.write(abi::REG_ZERO, 0);

        if self.trace {
            if self.privilege != prev_priv {
                trace_trap!(self.trace;
                    event      = "mode-switch",
                    from_mode  = prev_priv.name(),
                    to_mode    = self.privilege.name(),
                    pc         = %crate::trace::Hex(self.pc),
                    "CPU privilege mode switch"
                );
            }

            if self.stats.cycles.is_multiple_of(STATUS_UPDATE_INTERVAL) {
                ::tracing::debug!(
                    target: "rvsim::cpu",
                    cycles = self.stats.cycles,
                    pc     = %crate::trace::Hex(self.pc),
                    mode   = self.privilege.name(),
                    "CPU status"
                );
            }
        }
    }

    /// Tracks cycles spent in each privilege mode for statistics.
    const fn track_mode_cycles(&mut self) {
        match self.privilege {
            PrivilegeMode::User => self.stats.cycles_user += 1,
            PrivilegeMode::Supervisor => self.stats.cycles_kernel += 1,
            PrivilegeMode::Machine => self.stats.cycles_machine += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_track_mode_cycles() {
        let config = Config::default();
        let system = crate::soc::builder::System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.privilege = PrivilegeMode::User;
        cpu.track_mode_cycles();
        assert_eq!(cpu.stats.cycles_user, 1);

        cpu.privilege = PrivilegeMode::Supervisor;
        cpu.track_mode_cycles();
        assert_eq!(cpu.stats.cycles_kernel, 1);

        cpu.privilege = PrivilegeMode::Machine;
        cpu.track_mode_cycles();
        assert_eq!(cpu.stats.cycles_machine, 1);
    }

    #[test]
    fn test_post_tick_zero_reg() {
        let config = Config::default();
        let system = crate::soc::builder::System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.regs.write(abi::REG_ZERO, 42);
        cpu.post_tick(PrivilegeMode::Machine);
        assert_eq!(cpu.regs.read(abi::REG_ZERO), 0);
    }
}

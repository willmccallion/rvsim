//! Main Execution Loop — pre/post-tick orchestration of pipeline, interrupts, and cycles.

use super::SimState;
use crate::common::constants::{
    HANG_DETECTION_THRESHOLD, PAGE_OFFSET_MASK, PAGE_SHIFT, STATUS_UPDATE_INTERVAL, VPN_MASK,
    WFI_INSTRUCTION,
};
use crate::common::{Asid, SimError, Vpn};
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::isa::abi;
use crate::sim::stats::paths;
use crate::trace_trap;

impl SimState {
    /// Pre-tick: exit checks, interrupts, timers, cycle counting.
    ///
    /// Returns `Ok(true)` if the pipeline should be skipped this cycle
    /// (e.g. due to ALU timer stall or exit), `Ok(false)` to run the pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::KernelPanic`] when the bus panic sentinel fires.
    pub fn pre_tick(&mut self) -> Result<bool, SimError> {
        if self.check_exit().is_some() {
            return Ok(true);
        }

        let hart_idx = self.hart.hart_id.as_index();
        if self.bus.check_kernel_panic() {
            let detected_at =
                *self.per_hart_debug[hart_idx].panic_detected_at_cycle.get_or_insert(self.cycle);
            if self.cycle.saturating_sub(detected_at) >= 10_000 {
                return Err(SimError::KernelPanic { cycle: detected_at });
            }
        }

        let debug = &mut self.per_hart_debug[hart_idx];
        if self.hart.pc == debug.last_pc {
            debug.same_pc_count += 1;
            if debug.same_pc_count == HANG_DETECTION_THRESHOLD {
                let asid = Asid::new(
                    ((self.hart.csrs.satp >> csr::SATP_ASID_SHIFT) & csr::SATP_ASID_MASK) as u16,
                );
                // Hang detection reads the instruction at the stuck PC for
                // tracing; uses the RAM fast-path pointer (bench-side
                // observability — no cache modelling needed).
                let paddr_raw = if let Some(hit) =
                    self.hart.mmu.dtlb.lookup(Vpn::new((self.hart.pc >> PAGE_SHIFT) & VPN_MASK), asid)
                {
                    hit.ppn.to_addr() | (self.hart.pc & PAGE_OFFSET_MASK)
                } else {
                    self.hart.pc
                };
                let inst = self
                    .bus
                    .ram_region()
                    .filter(|r| r.contains(paddr_raw, 4))
                    .map_or(0u32, |r| {
                        // SAFETY: `RamRegion::contains` bounds-checks the access.
                        unsafe { r.ptr(paddr_raw).cast::<u32>().read_unaligned() }
                    });

                if inst == WFI_INSTRUCTION {
                    trace_trap!(self.config.general.trace_instructions;
                        event = "wfi-wait",
                        pc    = %crate::trace::Hex(self.hart.pc),
                        "CPU stuck in WFI — waiting for interrupt"
                    );
                } else {
                    trace_trap!(self.config.general.trace_instructions;
                        event = "potential-hang",
                        pc    = %crate::trace::Hex(self.hart.pc),
                        inst  = inst,
                        "CPU potential hang detected"
                    );
                }
            }
        } else {
            debug.last_pc = self.hart.pc;
            debug.same_pc_count = 0;
        }

        let irqs = self.bus_tick();

        let mut mip = self.hart.csrs.mip;

        if irqs.timer {
            mip |= csr::MIP_MTIP;
        } else {
            mip &= !csr::MIP_MTIP;
        }

        if irqs.msip {
            mip |= csr::MIP_MSIP;
        } else {
            mip &= !csr::MIP_MSIP;
        }

        if irqs.meip {
            mip |= csr::MIP_MEIP;
        } else {
            mip &= !csr::MIP_MEIP;
        }
        // SEIP is the logical-OR of the hardware signal (PLIC) and the
        // software-written bit.  Only clear the hardware component; preserve
        // the software-written bit so M-mode can inject S-mode external
        // interrupts via `csrw mip`.
        if irqs.seip {
            mip |= csr::MIP_SEIP;
        } else if !self.hart.sw_seip {
            mip &= !csr::MIP_SEIP;
        }

        // STIP management: when Sstc is enabled, hardware compares mtime
        // against stimecmp.  When Sstc is NOT active (the common case —
        // OpenSBI injects STIP via `csrw mip`), leave STIP entirely under
        // software control so that M-mode timer handlers work correctly.
        if (self.hart.csrs.menvcfg & csr::MENVCFG_STCE) != 0 {
            let mtime = self.cycle / self.config.system.clint_divider;
            if mtime >= self.hart.csrs.stimecmp {
                mip |= csr::MIP_STIP;
            } else {
                mip &= !csr::MIP_STIP;
            }
        }

        self.hart.csrs.mip = mip;

        self.cycle += 1;
        self.track_mode_cycles();

        Ok(false)
    }

    /// Post-tick: zero x0, privilege tracing, status printing.
    pub fn post_tick(&mut self, prev_priv: PrivilegeMode) {
        self.hart.regs.write(abi::REG_ZERO, 0);

        if self.config.general.trace_instructions {
            if self.hart.privilege != prev_priv {
                trace_trap!(self.config.general.trace_instructions;
                    event      = "mode-switch",
                    from_mode  = prev_priv.name(),
                    to_mode    = self.hart.privilege.name(),
                    pc         = %crate::trace::Hex(self.hart.pc),
                    "CPU privilege mode switch"
                );
            }

            if self.cycle.is_multiple_of(STATUS_UPDATE_INTERVAL) {
                ::tracing::debug!(
                    target: "rvsim::cpu",
                    cycles = self.cycle,
                    pc     = %crate::trace::Hex(self.hart.pc),
                    mode   = self.hart.privilege.name(),
                    "CPU status"
                );
            }
        }
    }

    /// Tracks cycles spent in each privilege mode for statistics.
    fn track_mode_cycles(&mut self) {
        match self.hart.privilege {
            PrivilegeMode::User => self.stats.counter(paths::hart::CYCLES_USER).inc(),
            PrivilegeMode::Supervisor => self.stats.counter(paths::hart::CYCLES_KERNEL).inc(),
            PrivilegeMode::Machine => self.stats.counter(paths::hart::CYCLES_MACHINE).inc(),
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
        let mut state = SimState::build(&config, "");

        state.hart.privilege = PrivilegeMode::User;
        state.track_mode_cycles();
        assert_eq!(state.stats.get(paths::hart::CYCLES_USER).unwrap_or(0.0) as u64, 1);

        state.hart.privilege = PrivilegeMode::Supervisor;
        state.track_mode_cycles();
        assert_eq!(state.stats.get(paths::hart::CYCLES_KERNEL).unwrap_or(0.0) as u64, 1);

        state.hart.privilege = PrivilegeMode::Machine;
        state.track_mode_cycles();
        assert_eq!(state.stats.get(paths::hart::CYCLES_MACHINE).unwrap_or(0.0) as u64, 1);
    }

    #[test]
    fn test_post_tick_zero_reg() {
        let config = Config::default();
        let mut state = SimState::build(&config, "");

        state.hart.regs.write(abi::REG_ZERO, 42);
        state.post_tick(PrivilegeMode::Machine);
        assert_eq!(state.hart.regs.read(abi::REG_ZERO), 0);
    }
}

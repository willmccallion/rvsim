//! Main Execution Loop — pre/post-tick orchestration of pipeline, interrupts, and cycles.

use super::{CoreCtx, SharedState};
use crate::arch::csr;
use crate::common::{Asid, PAGE_OFFSET_MASK, PAGE_SHIFT, SimError, VPN_MASK, Vpn};
use crate::isa::encoding::privileged::WFI;
use crate::isa::privileged::PrivilegeMode;
use crate::isa::reg;
use crate::soc::bus::HartIrqs;
use crate::trace_trap;

/// Cycles at one PC before the simulator reports a possible hang.
const HANG_DETECTION_THRESHOLD: u64 = 5000;

/// Cycles between progress reports.
const STATUS_UPDATE_INTERVAL: u64 = 5_000_000;

impl SharedState {
    /// Uncore work at the top of a cycle: exit and kernel-panic checks, then
    /// one tick of every bus device, which samples every hart's interrupt
    /// lines. Returns `false` when a device has requested exit and the
    /// cycle should be skipped.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::KernelPanic`] when the bus panic sentinel fires.
    pub fn pre_cycle(&mut self) -> Result<bool, SimError> {
        if self.check_exit().is_some() {
            return Ok(false);
        }

        if self.bus.check_kernel_panic() {
            let detected_at = *self.panic_detected_at_cycle.get_or_insert(self.cycle);
            if self.cycle.saturating_sub(detected_at) >= 10_000 {
                return Err(SimError::KernelPanic { cycle: detected_at });
            }
        }

        self.bus.tick();
        Ok(true)
    }

    /// Advances the master clock by one cycle.
    pub const fn advance_cycle(&mut self) {
        self.cycle += 1;
    }
}

impl CoreCtx<'_> {
    /// Counts a cycle an idle core spends in WFI without ticking its
    /// pipeline: the commit stage's zero-retire and WFI counts.
    pub fn count_idle_cycle(&mut self) {
        let paths = &self.core.stat_paths;
        self.shared.stats.counter(paths.commit.retire_hist_zero).inc();
        self.shared.stats.counter(paths.pipeline.cycles_wfi).inc();
    }

    /// Credits `cycles` cycles an idle core spends waiting with the whole
    /// system quiet, as that many ticks would: the hang detector's count,
    /// `mcycle`, the privilege-mode and core cycle counts, and the idle
    /// counts.
    pub fn skip_quiet_cycles(&mut self, cycles: u64) {
        let hart_idx = self.hart.hart_id.as_index();
        let debug = &mut self.shared.per_hart_debug[hart_idx];
        debug.same_pc_count = debug.same_pc_count.saturating_add(cycles);
        self.hart.csrs.count_cycles(cycles);
        let hart_paths = self.hart_paths();
        let mode_cycles = match self.hart.privilege {
            PrivilegeMode::User => hart_paths.cycles_user,
            PrivilegeMode::Supervisor => hart_paths.cycles_kernel,
            PrivilegeMode::Machine => hart_paths.cycles_machine,
        };
        let paths = &self.core.stat_paths;
        let stats = &mut self.shared.stats;
        stats.counter(mode_cycles).add(cycles);
        stats.counter(paths.pipeline.cycles_total).add(cycles);
        stats.counter(paths.commit.retire_hist_zero).add(cycles);
        stats.counter(paths.pipeline.cycles_wfi).add(cycles);
    }

    /// Per-hart work at the top of a cycle, before the clock advances:
    /// hang detection and folding this hart's interrupt lines into `mip`.
    pub fn pre_tick(&mut self, irqs: HartIrqs) {
        let hart_idx = self.hart.hart_id.as_index();
        let debug = &mut self.shared.per_hart_debug[hart_idx];
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
                    self.core.mmu.dtlb.peek(Vpn::new((self.hart.pc >> PAGE_SHIFT) & VPN_MASK), asid)
                {
                    hit.ppn.to_addr() | (self.hart.pc & PAGE_OFFSET_MASK)
                } else {
                    self.hart.pc
                };
                let inst =
                    self.bus.ram_region().filter(|r| r.contains(paddr_raw, 4)).map_or(0u32, |r| {
                        // SAFETY: `RamRegion::contains` bounds-checks the access.
                        unsafe { r.ptr(paddr_raw).cast::<u32>().read_unaligned() }
                    });

                if inst == WFI {
                    trace_trap!(self.config.general.trace_instructions;
                        event = "wfi-wait",
                        pc    = %crate::sim::trace::Hex(self.hart.pc),
                        "CPU stuck in WFI — waiting for interrupt"
                    );
                } else {
                    trace_trap!(self.config.general.trace_instructions;
                        event = "potential-hang",
                        pc    = %crate::sim::trace::Hex(self.hart.pc),
                        inst  = inst,
                        "CPU potential hang detected"
                    );
                }
            }
        } else {
            debug.last_pc = self.hart.pc;
            debug.same_pc_count = 0;
        }

        let mut mip = self.hart.csrs.mip;

        if irqs.mtip {
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

        // STIP management: when Sstc is enabled, hardware compares `time`
        // (the CLINT's mtime) against stimecmp. Without it, OpenSBI injects
        // STIP via `csrw mip`, so STIP stays under software control.
        if (self.hart.csrs.menvcfg & csr::MENVCFG_STCE) != 0 {
            if self.bus.mtime() >= self.hart.csrs.stimecmp {
                mip |= csr::MIP_STIP;
            } else {
                mip &= !csr::MIP_STIP;
            }
        }

        self.hart.csrs.mip = mip;
    }

    /// Post-tick: zero x0, privilege tracing, status printing.
    pub fn post_tick(&mut self, prev_priv: PrivilegeMode) {
        self.hart.regs.write(reg::REG_ZERO, 0);

        if self.config.general.trace_instructions {
            if self.hart.privilege != prev_priv {
                trace_trap!(self.config.general.trace_instructions;
                    event      = "mode-switch",
                    from_mode  = prev_priv.name(),
                    to_mode    = self.hart.privilege.name(),
                    pc         = %crate::sim::trace::Hex(self.hart.pc),
                    "CPU privilege mode switch"
                );
            }

            if self.cycle.is_multiple_of(STATUS_UPDATE_INTERVAL) {
                ::tracing::debug!(
                    target: "rvsim::cpu",
                    cycles = self.cycle,
                    pc     = %crate::sim::trace::Hex(self.hart.pc),
                    mode   = self.hart.privilege.name(),
                    "CPU status"
                );
            }
        }
    }

    /// Charges the cycle that just began to the hart's current privilege
    /// mode.
    pub fn track_mode_cycles(&mut self) {
        self.hart.csrs.count_cycle();
        let hart_paths = self.hart_paths();
        let core_cycles = self.core.stat_paths.pipeline.cycles_total;
        self.stats.counter(core_cycles).inc();
        match self.hart.privilege {
            PrivilegeMode::User => self.stats.counter(hart_paths.cycles_user).inc(),
            PrivilegeMode::Supervisor => self.stats.counter(hart_paths.cycles_kernel).inc(),
            PrivilegeMode::Machine => self.stats.counter(hart_paths.cycles_machine).inc(),
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
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let paths = state.hart_paths();
        state.hart.privilege = PrivilegeMode::User;
        state.track_mode_cycles();
        assert_eq!(state.stats.get(paths.cycles_user).unwrap_or(0.0) as u64, 1);

        state.hart.privilege = PrivilegeMode::Supervisor;
        state.track_mode_cycles();
        assert_eq!(state.stats.get(paths.cycles_kernel).unwrap_or(0.0) as u64, 1);

        state.hart.privilege = PrivilegeMode::Machine;
        state.track_mode_cycles();
        assert_eq!(state.stats.get(paths.cycles_machine).unwrap_or(0.0) as u64, 1);
        assert_eq!(
            state.stats.get(state.core.stat_paths.pipeline.cycles_total).unwrap_or(0.0) as u64,
            3
        );
    }

    #[test]
    fn test_post_tick_zero_reg() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.regs.write(reg::REG_ZERO, 42);
        state.post_tick(PrivilegeMode::Machine);
        assert_eq!(state.hart.regs.read(reg::REG_ZERO), 0);
    }
}

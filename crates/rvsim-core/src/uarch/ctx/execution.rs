//! Main Execution Loop — pre/post-tick orchestration of pipeline, interrupts, and cycles.

use super::CoreCtx;
use crate::common::PhysAddr;
use crate::common::trace::Hex;
use crate::isa::csr;
use crate::isa::encoding::privileged::WFI;
use crate::isa::privileged::{PrivilegeMode, Satp};
use crate::isa::reg;
use crate::soc::bus::HartIrqs;
use crate::trace_trap;

/// Cycles at one PC before the simulator reports a possible hang.
const HANG_DETECTION_THRESHOLD: u64 = 5000;

/// Cycles between progress reports.
const STATUS_UPDATE_INTERVAL: u64 = 5_000_000;

impl CoreCtx<'_> {
    /// Counts a cycle an idle core spends in WFI without ticking its
    /// pipeline: the commit stage's zero-retire and WFI counts.
    pub fn count_idle_cycle(&mut self) {
        let paths = &self.core.stat_paths;
        self.uncore.kernel.stats.counter(paths.commit.retire_hist_zero).inc();
        self.uncore.kernel.stats.counter(paths.pipeline.cycles_wfi).inc();
    }

    /// Credits `cycles` cycles an idle core spends waiting with the whole
    /// system quiet, as that many ticks would: the hang detector's count,
    /// `mcycle`, the privilege-mode and core cycle counts, and the idle
    /// counts.
    pub fn skip_quiet_cycles(&mut self, cycles: u64) {
        let hart_idx = self.hart.hart_id.as_index();
        let debug = &mut self.uncore.per_hart_debug[hart_idx];
        debug.same_pc_count = debug.same_pc_count.saturating_add(cycles);
        self.hart.csrs.count_cycles(cycles);
        let hart_paths = self.hart_paths();
        let mode_cycles = match self.hart.privilege {
            PrivilegeMode::User => hart_paths.cycles_user,
            PrivilegeMode::Supervisor => hart_paths.cycles_kernel,
            PrivilegeMode::Machine => hart_paths.cycles_machine,
        };
        let paths = &self.core.stat_paths;
        let stats = &mut self.uncore.kernel.stats;
        stats.counter(mode_cycles).add(cycles);
        stats.counter(paths.pipeline.cycles_total).add(cycles);
        stats.counter(paths.commit.retire_hist_zero).add(cycles);
        stats.counter(paths.pipeline.cycles_wfi).add(cycles);
    }

    /// Per-hart work at the top of a cycle, before the clock advances:
    /// hang detection and folding this hart's interrupt lines into `mip`.
    pub fn pre_tick(&mut self, irqs: HartIrqs) {
        self.detect_hang();
        self.update_mip(irqs);
    }

    /// Counts cycles at one PC and, at the threshold, traces whether the
    /// hart waits in WFI or may be hung.
    fn detect_hang(&mut self) {
        let hart_idx = self.hart.hart_id.as_index();
        let debug = &mut self.uncore.per_hart_debug[hart_idx];
        if self.hart.pc != debug.last_pc {
            debug.last_pc = self.hart.pc;
            debug.same_pc_count = 0;
            return;
        }
        debug.same_pc_count += 1;
        if debug.same_pc_count != HANG_DETECTION_THRESHOLD {
            return;
        }
        let asid = Satp::new(self.hart.csrs.satp).asid();
        // Hang detection reads the instruction at the stuck PC for
        // tracing straight from the memory image (bench-side
        // observability — no cache modelling needed).
        let paddr_raw = if let Some(hit) = self.core.mmu.dtlb.peek(self.hart.pc.page_number(), asid)
        {
            hit.ppn.to_addr() | self.hart.pc.page_offset()
        } else {
            self.hart.pc.val()
        };
        let inst = self.uncore.kernel.memory.read(PhysAddr::new(paddr_raw), 4).unwrap_or(0) as u32;
        if inst == WFI {
            trace_trap!(self.uncore.kernel.config.general.trace_instructions;
                event = "wfi-wait",
                pc    = %Hex(self.hart.pc.val()),
                "CPU stuck in WFI — waiting for interrupt"
            );
        } else {
            trace_trap!(self.uncore.kernel.config.general.trace_instructions;
                event = "potential-hang",
                pc    = %Hex(self.hart.pc.val()),
                inst  = inst,
                "CPU potential hang detected"
            );
        }
    }

    /// Folds the interrupt lines `irqs` into `mip`.
    fn update_mip(&mut self, irqs: HartIrqs) {
        // With Sstc, hardware compares `time` (the CLINT's mtime) against
        // stimecmp. Without it, OpenSBI injects STIP via `csrw mip`, so STIP
        // stays under software control.
        let sstc = (self.hart.csrs.menvcfg & csr::MENVCFG_STCE) != 0;
        let stip = sstc.then(|| self.uncore.bus.mtime() >= self.hart.csrs.stimecmp);
        let sw_seip = self.hart.sw_seip;
        let mip = &mut self.hart.csrs.mip;
        set_bits(mip, csr::MIP_MTIP, irqs.mtip);
        set_bits(mip, csr::MIP_MSIP, irqs.msip);
        set_bits(mip, csr::MIP_MEIP, irqs.meip);
        // SEIP is the logical-OR of the hardware signal (PLIC) and the
        // software-written bit: a low line leaves a software-set bit alone,
        // so M-mode can inject S-mode external interrupts via `csrw mip`.
        if irqs.seip || !sw_seip {
            set_bits(mip, csr::MIP_SEIP, irqs.seip);
        }
        if let Some(due) = stip {
            set_bits(mip, csr::MIP_STIP, due);
        }
    }

    /// Post-tick: zero x0, privilege tracing, status printing.
    pub fn post_tick(&mut self, prev_priv: PrivilegeMode) {
        self.hart.regs.write(reg::REG_ZERO, 0);

        if self.uncore.kernel.config.general.trace_instructions {
            if self.hart.privilege != prev_priv {
                trace_trap!(self.uncore.kernel.config.general.trace_instructions;
                    event      = "mode-switch",
                    from_mode  = prev_priv.name(),
                    to_mode    = self.hart.privilege.name(),
                    pc         = %Hex(self.hart.pc.val()),
                    "CPU privilege mode switch"
                );
            }

            if self.uncore.kernel.cycle.is_multiple_of(STATUS_UPDATE_INTERVAL) {
                ::tracing::debug!(
                    target: "rvsim::cpu",
                    cycles = self.uncore.kernel.cycle,
                    pc     = %Hex(self.hart.pc.val()),
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
        self.uncore.kernel.stats.counter(core_cycles).inc();
        let mode_cycles = match self.hart.privilege {
            PrivilegeMode::User => hart_paths.cycles_user,
            PrivilegeMode::Supervisor => hart_paths.cycles_kernel,
            PrivilegeMode::Machine => hart_paths.cycles_machine,
        };
        self.uncore.kernel.stats.counter(mode_cycles).inc();
    }
}

/// Sets `mask`'s bits in `word` when `on`, else clears them.
const fn set_bits(word: &mut u64, mask: u64, on: bool) {
    if on {
        *word |= mask;
    } else {
        *word &= !mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_track_mode_cycles() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        let paths = state.hart_paths();
        state.hart.privilege = PrivilegeMode::User;
        state.track_mode_cycles();
        assert_eq!(state.uncore.kernel.stats.get(paths.cycles_user).unwrap_or(0.0) as u64, 1);

        state.hart.privilege = PrivilegeMode::Supervisor;
        state.track_mode_cycles();
        assert_eq!(state.uncore.kernel.stats.get(paths.cycles_kernel).unwrap_or(0.0) as u64, 1);

        state.hart.privilege = PrivilegeMode::Machine;
        state.track_mode_cycles();
        assert_eq!(state.uncore.kernel.stats.get(paths.cycles_machine).unwrap_or(0.0) as u64, 1);
        assert_eq!(
            state
                .uncore
                .kernel
                .stats
                .get(state.core.stat_paths.pipeline.cycles_total)
                .unwrap_or(0.0) as u64,
            3
        );
    }

    #[test]
    fn test_post_tick_zero_reg() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.hart.regs.write(reg::REG_ZERO, 42);
        state.post_tick(PrivilegeMode::Machine);
        assert_eq!(state.hart.regs.read(reg::REG_ZERO), 0);
    }
}

//! Trap and exception dispatch, delegation, and MRET/SRET return handling.

use super::CoreCtx;
use crate::isa::csr;
use crate::isa::privileged::cause::CAUSE_INTERRUPT_BIT;
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::trace_trap;

impl CoreCtx<'_> {
    /// Handles a trap (exception or interrupt).
    pub fn trap(&mut self, cause: &Trap, epc: u64) {
        self.clear_reservation();

        if self.uncore.processes.get(self.hart.hart_id).is_some() {
            // There is no kernel to take it: the program dies as it would
            // under Linux, with the signal's exit status.
            let signal = match cause {
                Trap::IllegalInstruction(_) => 4,
                Trap::Breakpoint(_) => 5,
                Trap::InstructionAddressMisaligned(_)
                | Trap::LoadAddressMisaligned(_)
                | Trap::StoreAddressMisaligned(_) => 7,
                _ => 11,
            };
            tracing::error!(target: "rvsim::trap", ?cause, pc = %crate::common::trace::Hex(epc), "the program faulted in SE mode");
            // Left for a post-mortem: what faulted, where, and on what.
            let (_, code) = cause.cause();
            self.hart.csrs.mcause = code;
            self.hart.csrs.mepc = epc;
            self.hart.csrs.mtval = match *cause {
                Trap::InstructionAddressMisaligned(a)
                | Trap::InstructionAccessFault(a)
                | Trap::Breakpoint(a)
                | Trap::LoadAddressMisaligned(a)
                | Trap::LoadAccessFault(a)
                | Trap::StoreAddressMisaligned(a)
                | Trap::StoreAccessFault(a)
                | Trap::InstructionPageFault(a)
                | Trap::LoadPageFault(a)
                | Trap::StorePageFault(a) => a,
                Trap::IllegalInstruction(i) => u64::from(i),
                _ => 0,
            };
            self.exit_process(128 + signal);
            return;
        }

        let (is_interrupt, code) = cause.cause();
        {
            trace_trap!(self.trace_trap_enabled(cause);
                event      = "taken",
                epc        = %crate::common::trace::Hex(epc),
                cause      = ?cause,
                priv_mode  = ?self.hart.privilege,
                stvec      = %crate::common::trace::Hex(self.hart.csrs.stvec),
                mtvec      = %crate::common::trace::Hex(self.hart.csrs.mtvec),
                "trap taken"
            );
        }

        let deleg_mask = if is_interrupt { self.hart.csrs.mideleg } else { self.hart.csrs.medeleg };
        let delegate_to_s =
            (self.hart.privilege <= PrivilegeMode::Supervisor) && ((deleg_mask >> code) & 1) != 0;

        let tval = match *cause {
            Trap::InstructionAddressMisaligned(a)
            | Trap::InstructionAccessFault(a)
            | Trap::Breakpoint(a)
            | Trap::LoadAddressMisaligned(a)
            | Trap::LoadAccessFault(a)
            | Trap::StoreAddressMisaligned(a)
            | Trap::StoreAccessFault(a)
            | Trap::InstructionPageFault(a)
            | Trap::LoadPageFault(a)
            | Trap::StorePageFault(a) => a,
            Trap::IllegalInstruction(i) => i as u64,
            _ => 0,
        };

        if delegate_to_s {
            self.hart.csrs.scause = if is_interrupt { CAUSE_INTERRUPT_BIT | code } else { code };

            self.hart.csrs.sepc = epc;
            self.hart.csrs.stval = tval;

            let mut mstatus = self.hart.csrs.mstatus;
            if (mstatus & csr::MSTATUS_SIE) != 0 {
                mstatus |= csr::MSTATUS_SPIE;
            } else {
                mstatus &= !csr::MSTATUS_SPIE;
            }
            if self.hart.privilege == PrivilegeMode::Supervisor {
                mstatus |= csr::MSTATUS_SPP;
            } else {
                mstatus &= !csr::MSTATUS_SPP;
            }
            mstatus &= !csr::MSTATUS_SIE;
            self.hart.csrs.mstatus = mstatus;

            self.hart.privilege = PrivilegeMode::Supervisor;
            let stvec_base = self.hart.csrs.stvec & !3;
            let trap_handler_pc = stvec_base
                + (if (self.hart.csrs.stvec & 1) != 0 && is_interrupt { 4 * code } else { 0 });

            self.hart.pc = trap_handler_pc;
        } else {
            self.hart.csrs.mcause = if is_interrupt { CAUSE_INTERRUPT_BIT | code } else { code };
            self.hart.csrs.mepc = epc;
            self.hart.csrs.mtval = tval;

            let mut mstatus = self.hart.csrs.mstatus;
            if (mstatus & csr::MSTATUS_MIE) != 0 {
                mstatus |= csr::MSTATUS_MPIE;
            } else {
                mstatus &= !csr::MSTATUS_MPIE;
            }
            mstatus &= !csr::MSTATUS_MPP;
            mstatus |= (self.hart.privilege.to_u8() as u64) << csr::MSTATUS_MPP_SHIFT;
            mstatus &= !csr::MSTATUS_MIE;
            self.hart.csrs.mstatus = mstatus;

            self.hart.privilege = PrivilegeMode::Machine;
            let mtvec_base = self.hart.csrs.mtvec & !3;
            let target_pc = mtvec_base
                + (if (self.hart.csrs.mtvec & 1) != 0 && is_interrupt { 4 * code } else { 0 });
            self.hart.pc = target_pc;
        }

        #[cfg(feature = "commit-log")]
        if let Some(log) = self.uncore.commit_log.as_mut() {
            let cause_bits = if is_interrupt { CAUSE_INTERRUPT_BIT | code } else { code };
            let _ = crate::uarch::pipeline::commit_log::write_trap(
                log,
                cause_bits,
                epc,
                tval,
                self.uncore.cycle,
            );
        }

        let hart_paths = self.hart_paths();
        self.stats.counter(hart_paths.traps).inc();
    }

    /// Executes the `MRET` instruction (Return from Machine Mode).
    #[inline]
    pub(crate) fn do_mret(&mut self) {
        self.clear_reservation();
        self.hart.do_mret();
    }

    /// Executes the `SRET` instruction (Return from Supervisor Mode).
    #[inline]
    pub(crate) fn do_sret(&mut self) {
        self.clear_reservation();
        self.hart.do_sret();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_do_mret() {
        let config = Config::default();
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.csrs.mepc = 0x2000;
        state.hart.csrs.mstatus =
            (PrivilegeMode::Supervisor.to_u8() as u64) << csr::MSTATUS_MPP_SHIFT;
        state.hart.csrs.mstatus |= csr::MSTATUS_MPIE;

        state.do_mret();

        assert_eq!(state.hart.pc, 0x2000);
        assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
        assert_eq!(state.hart.csrs.mstatus & csr::MSTATUS_MIE, csr::MSTATUS_MIE);
    }

    #[test]
    fn test_do_sret() {
        let config = Config::default();
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.csrs.sepc = 0x3000;
        state.hart.csrs.mstatus = csr::MSTATUS_SPP | csr::MSTATUS_SPIE;

        state.do_sret();

        assert_eq!(state.hart.pc, 0x3000);
        assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
        assert_eq!(state.hart.csrs.mstatus & csr::MSTATUS_SIE, csr::MSTATUS_SIE);
    }
}

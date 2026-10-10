//! Trap and exception dispatch, delegation, and MRET/SRET return handling.

use super::CoreCtx;
use crate::isa::csr;
use crate::isa::encoding::privileged as sys_ops;
use crate::isa::privileged::cause::CAUSE_INTERRUPT_BIT;
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::isa::reg;
use crate::trace_trap;

impl CoreCtx<'_> {
    /// Handles a trap (exception or interrupt).
    pub fn trap(&mut self, cause: &Trap, epc: u64) {
        self.clear_reservation();

        if self.direct_mode && self.hart.csrs.mtvec == 0 {
            // In direct mode with no trap handler installed (mtvec == 0),
            // handle traps directly: ecall triggers SYS_EXIT and everything
            // else is fatal.  When mtvec has been written (e.g. by arch-test
            // bootstrap code), we fall through to the standard dispatch path
            // so that the installed handler runs normally.
            if matches!(
                cause,
                Trap::EnvironmentCallFromUMode
                    | Trap::EnvironmentCallFromSMode
                    | Trap::EnvironmentCallFromMMode
            ) {
                let val_a7 = self.hart.regs.read(reg::REG_A7);
                let val_a0 = self.hart.regs.read(reg::REG_A0);

                if val_a7 == sys_ops::SYS_EXIT {
                    self.signal_exit(val_a0);
                    return;
                } else if val_a0 == sys_ops::SYS_EXIT {
                    let val_a1 = self.hart.regs.read(reg::REG_A1);
                    self.signal_exit(val_a1);
                    return;
                }

                tracing::error!(target: "rvsim::trap", a7 = val_a7, a0 = val_a0, pc = epc, "unhandled ecall in direct mode");
                self.signal_exit(1);
                return;
            }

            if matches!(cause, Trap::IllegalInstruction(0)) {
                self.signal_exit(0);
                return;
            }
            tracing::error!(target: "rvsim::trap", ?cause, pc = epc, "fatal trap in direct mode");
            self.signal_exit(1);
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
    fn test_trap_direct_mode_ecall() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.regs.write(reg::REG_A7, sys_ops::SYS_EXIT);
        state.hart.regs.write(reg::REG_A0, 42);

        state.trap(&Trap::EnvironmentCallFromMMode, 0x1000);
        assert_eq!(state.check_exit(), Some(42));
    }

    #[test]
    fn test_trap_direct_mode_illegal_instruction() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.trap(&Trap::IllegalInstruction(0), 0x1000);
        assert_eq!(state.check_exit(), Some(0));
    }

    #[test]
    fn test_trap_direct_mode_breakpoint_with_mtvec() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.csrs.mtvec = 0x8000_1000;
        state.trap(&Trap::Breakpoint(0x400), 0x400);

        assert!(state.check_exit().is_none(), "should not be fatal when mtvec is set");
        assert_eq!(state.hart.csrs.mepc, 0x400);
        assert_eq!(state.hart.csrs.mcause, 3);
        assert_eq!(state.hart.csrs.mtval, 0x400);
        assert_eq!(state.hart.pc, 0x8000_1000);
    }

    #[test]
    fn test_trap_direct_mode_breakpoint_no_mtvec() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.trap(&Trap::Breakpoint(0x400), 0x400);
        assert_eq!(state.check_exit(), Some(1));
    }

    #[test]
    fn test_trap_direct_mode_ecall_with_mtvec() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.csrs.mtvec = 0x8000_2000;
        state.trap(&Trap::EnvironmentCallFromMMode, 0x500);

        assert!(state.check_exit().is_none());
        assert_eq!(state.hart.csrs.mepc, 0x500);
        assert_eq!(state.hart.pc, 0x8000_2000);
    }

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

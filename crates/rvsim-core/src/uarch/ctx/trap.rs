//! Trap and exception dispatch, delegation, and MRET/SRET return handling.

use super::CoreCtx;
use crate::common::VirtAddr;
use crate::isa::csr;
use crate::isa::privileged::{PrivilegeMode, Trap, TrapCause};
use crate::trace_trap;

/// Where a trap with `cause` enters through `tvec`: its base, plus four
/// bytes per interrupt code in vectored mode.
const fn handler_pc(tvec: u64, cause: TrapCause) -> VirtAddr {
    let base = tvec & !3;
    VirtAddr::new(match cause {
        TrapCause::Interrupt(code) if tvec & 1 != 0 => base + 4 * code,
        _ => base,
    })
}

/// The Linux signals a user program's fault raises.
const SIGILL: u8 = 4;
const SIGTRAP: u8 = 5;
const SIGBUS: u8 = 7;
const SIGSEGV: u8 = 11;

/// The signal Linux sends a user program that takes `trap`.
const fn fatal_signal(trap: &Trap) -> u8 {
    match trap {
        Trap::IllegalInstruction(_) => SIGILL,
        Trap::Breakpoint(_) => SIGTRAP,
        Trap::InstructionAddressMisaligned(_)
        | Trap::LoadAddressMisaligned(_)
        | Trap::StoreAddressMisaligned(_) => SIGBUS,
        _ => SIGSEGV,
    }
}

impl CoreCtx<'_> {
    /// Handles a trap (exception or interrupt).
    pub fn trap(&mut self, cause: &Trap, epc: VirtAddr) {
        self.clear_reservation();
        if self.uncore.processes.is_se() {
            self.end_program_on_trap(cause, epc);
            return;
        }

        trace_trap!(self.trace_trap_enabled(cause);
            event      = "taken",
            epc        = %crate::common::trace::Hex(epc.val()),
            cause      = ?cause,
            priv_mode  = ?self.hart.privilege,
            stvec      = %crate::common::trace::Hex(self.hart.csrs.stvec),
            mtvec      = %crate::common::trace::Hex(self.hart.csrs.mtvec),
            "trap taken"
        );
        let trap_cause = cause.cause();
        let deleg_mask =
            if trap_cause.is_interrupt() { self.hart.csrs.mideleg } else { self.hart.csrs.medeleg };
        let delegate_to_s = (self.hart.privilege <= PrivilegeMode::Supervisor)
            && ((deleg_mask >> trap_cause.code()) & 1) != 0;
        if delegate_to_s {
            self.enter_supervisor(cause, epc);
        } else {
            self.enter_machine(cause, epc);
        }

        #[cfg(feature = "commit-log")]
        if let Some(log) = self.uncore.commit_log.as_mut() {
            let _ = crate::uarch::pipeline::commit_log::write_trap(
                log,
                cause.mcause_code(),
                epc,
                cause.tval(),
                self.uncore.cycle,
            );
        }

        let hart_paths = self.hart_paths();
        self.stats.counter(hart_paths.traps).inc();
    }

    /// An SE-mode program has no kernel to take its trap: it dies as it
    /// would under Linux, by the signal the trap raises. What faulted,
    /// where and on what are left in the machine CSRs for a post-mortem.
    fn end_program_on_trap(&mut self, cause: &Trap, epc: VirtAddr) {
        tracing::error!(target: "rvsim::trap", ?cause, pc = %crate::common::trace::Hex(epc.val()), "the program faulted in SE mode");
        self.hart.csrs.mcause = cause.cause().code();
        self.hart.csrs.mepc = epc.val();
        self.hart.csrs.mtval = cause.tval();
        self.end_process_by_signal(fatal_signal(cause));
    }

    /// Takes `cause` into supervisor mode at `stvec`.
    fn enter_supervisor(&mut self, cause: &Trap, epc: VirtAddr) {
        self.hart.csrs.scause = cause.mcause_code();
        self.hart.csrs.sepc = epc.val();
        self.hart.csrs.stval = cause.tval();

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
        self.hart.pc = handler_pc(self.hart.csrs.stvec, cause.cause());
    }

    /// Takes `cause` into machine mode at `mtvec`.
    const fn enter_machine(&mut self, cause: &Trap, epc: VirtAddr) {
        self.hart.csrs.mcause = cause.mcause_code();
        self.hart.csrs.mepc = epc.val();
        self.hart.csrs.mtval = cause.tval();

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
        self.hart.pc = handler_pc(self.hart.csrs.mtvec, cause.cause());
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
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.hart.csrs.mepc = 0x2000;
        state.hart.csrs.mstatus =
            (PrivilegeMode::Supervisor.to_u8() as u64) << csr::MSTATUS_MPP_SHIFT;
        state.hart.csrs.mstatus |= csr::MSTATUS_MPIE;

        state.do_mret();

        assert_eq!(state.hart.pc, VirtAddr::new(0x2000));
        assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
        assert_eq!(state.hart.csrs.mstatus & csr::MSTATUS_MIE, csr::MSTATUS_MIE);
    }

    #[test]
    fn test_do_sret() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.hart.csrs.sepc = 0x3000;
        state.hart.csrs.mstatus = csr::MSTATUS_SPP | csr::MSTATUS_SPIE;

        state.do_sret();

        assert_eq!(state.hart.pc, VirtAddr::new(0x3000));
        assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
        assert_eq!(state.hart.csrs.mstatus & csr::MSTATUS_SIE, csr::MSTATUS_SIE);
    }
}

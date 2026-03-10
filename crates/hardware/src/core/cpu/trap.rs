//! Trap Handling Logic.
//!
//! This module implements the trap and exception handling logic for the CPU. It performs
//! the following:
//! 1. **Trap Dispatch:** Identifies the trap cause and determines the appropriate handler mode.
//! 2. **Delegation:** Handles the delegation of traps from Machine mode to Supervisor mode.
//! 3. **Context Saving:** Updates CSRs (`mepc`, `mcause`, `mtval`, etc.) and modifies privilege state.
//! 4. **Return Handling:** Implements `MRET` and `SRET` instructions for returning from trap handlers.

use super::Cpu;
use crate::common::Trap;
use crate::common::constants::CAUSE_INTERRUPT_BIT;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::isa::abi;
use crate::isa::privileged::cause::{exception, interrupt};
use crate::isa::privileged::opcodes as sys_ops;
use crate::trace_trap;

impl Cpu {
    /// Handles a trap (exception or interrupt).
    ///
    /// # Arguments
    ///
    /// * `cause` - The type of trap that occurred.
    /// * `epc` - The Exception Program Counter (PC where the trap occurred).
    pub fn trap(&mut self, cause: &Trap, epc: u64) {
        self.load_reservation = None;

        if self.direct_mode {
            // In direct mode, ecall is handled here at commit time so that
            // all preceding instructions have retired and the architectural
            // register file contains the correct syscall arguments.
            if matches!(
                cause,
                Trap::EnvironmentCallFromUMode
                    | Trap::EnvironmentCallFromSMode
                    | Trap::EnvironmentCallFromMMode
            ) {
                let val_a7 = self.regs.read(abi::REG_A7);
                let val_a0 = self.regs.read(abi::REG_A0);

                if val_a7 == sys_ops::SYS_EXIT {
                    self.exit_code = Some(val_a0);
                    return;
                } else if val_a0 == sys_ops::SYS_EXIT {
                    let val_a1 = self.regs.read(abi::REG_A1);
                    self.exit_code = Some(val_a1);
                    return;
                }

                // Unknown syscall in direct mode — treat as fatal.
                eprintln!(
                    "\n[!] Unhandled ecall in direct mode: a7={val_a7} a0={val_a0} at PC {epc:#x}"
                );
                self.exit_code = Some(1);
                return;
            }

            // Non-ecall traps in direct mode are fatal.
            if matches!(cause, Trap::IllegalInstruction(0)) {
                self.exit_code = Some(0);
                return;
            }
            eprintln!("\n[!] Fatal trap in direct mode: {cause:?} at PC {epc:#x}");
            self.exit_code = Some(1);
            return;
        }

        let is_timer =
            matches!(cause, Trap::MachineTimerInterrupt | Trap::SupervisorTimerInterrupt);
        let is_ecall = matches!(
            cause,
            Trap::EnvironmentCallFromUMode
                | Trap::EnvironmentCallFromSMode
                | Trap::EnvironmentCallFromMMode
        );

        if !is_timer && !is_ecall {
            trace_trap!(self.trace;
                event      = "taken",
                epc        = %crate::trace::Hex(epc),
                cause      = ?cause,
                priv_mode  = ?self.privilege,
                stvec      = %crate::trace::Hex(self.csrs.stvec),
                mtvec      = %crate::trace::Hex(self.csrs.mtvec),
                "trap taken"
            );
        }

        let (is_interrupt, code) = match *cause {
            Trap::InstructionAddressMisaligned(_) => {
                (false, exception::INSTRUCTION_ADDRESS_MISALIGNED)
            }
            Trap::InstructionAccessFault(_) => (false, exception::INSTRUCTION_ACCESS_FAULT),
            Trap::IllegalInstruction(_) => (false, exception::ILLEGAL_INSTRUCTION),
            Trap::Breakpoint(_) => (false, exception::BREAKPOINT),
            Trap::LoadAddressMisaligned(_) => (false, exception::LOAD_ADDRESS_MISALIGNED),
            Trap::LoadAccessFault(_) => (false, exception::LOAD_ACCESS_FAULT),
            Trap::StoreAddressMisaligned(_) => (false, exception::STORE_ADDRESS_MISALIGNED),
            Trap::StoreAccessFault(_) => (false, exception::STORE_ACCESS_FAULT),
            Trap::EnvironmentCallFromUMode => (false, exception::ENVIRONMENT_CALL_FROM_U_MODE),
            Trap::EnvironmentCallFromSMode => (false, exception::ENVIRONMENT_CALL_FROM_S_MODE),
            Trap::EnvironmentCallFromMMode => (false, exception::ENVIRONMENT_CALL_FROM_M_MODE),
            Trap::InstructionPageFault(_) => (false, exception::INSTRUCTION_PAGE_FAULT),
            Trap::LoadPageFault(_) => (false, exception::LOAD_PAGE_FAULT),
            Trap::StorePageFault(_) => (false, exception::STORE_PAGE_FAULT),
            Trap::UserSoftwareInterrupt => (true, interrupt::USER_SOFTWARE & !CAUSE_INTERRUPT_BIT),
            Trap::SupervisorSoftwareInterrupt => {
                (true, interrupt::SUPERVISOR_SOFTWARE & !CAUSE_INTERRUPT_BIT)
            }
            Trap::MachineSoftwareInterrupt => {
                (true, interrupt::MACHINE_SOFTWARE & !CAUSE_INTERRUPT_BIT)
            }
            Trap::SupervisorTimerInterrupt => {
                (true, interrupt::SUPERVISOR_TIMER & !CAUSE_INTERRUPT_BIT)
            }
            Trap::MachineTimerInterrupt => (true, interrupt::MACHINE_TIMER & !CAUSE_INTERRUPT_BIT),
            Trap::UserExternalInterrupt => (true, interrupt::USER_EXTERNAL & !CAUSE_INTERRUPT_BIT),
            Trap::SupervisorExternalInterrupt => {
                (true, interrupt::SUPERVISOR_EXTERNAL & !CAUSE_INTERRUPT_BIT)
            }
            Trap::MachineExternalInterrupt => {
                (true, interrupt::MACHINE_EXTERNAL & !CAUSE_INTERRUPT_BIT)
            }
            Trap::RequestedTrap(c) => (false, c),
            Trap::DoubleFault(_) => (false, exception::HARDWARE_ERROR),
        };

        let deleg_mask = if is_interrupt { self.csrs.mideleg } else { self.csrs.medeleg };
        let delegate_to_s =
            (self.privilege <= PrivilegeMode::Supervisor) && ((deleg_mask >> code) & 1) != 0;

        // Delegation is determined solely by medeleg/mideleg per the RISC-V
        // privileged spec.  A previous workaround forced delegation to S-mode
        // when stvec was set; this was spec-violating and has been removed.

        let tval = match *cause {
            Trap::InstructionAddressMisaligned(a)
            | Trap::InstructionAccessFault(a)
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
            self.csrs.scause = if is_interrupt { CAUSE_INTERRUPT_BIT | code } else { code };

            self.csrs.sepc = epc;
            self.csrs.stval = tval;

            let mut sstatus = self.csrs.sstatus;
            if (sstatus & csr::MSTATUS_SIE) != 0 {
                sstatus |= csr::MSTATUS_SPIE;
            } else {
                sstatus &= !csr::MSTATUS_SPIE;
            }
            if self.privilege == PrivilegeMode::Supervisor {
                sstatus |= csr::MSTATUS_SPP;
            } else {
                sstatus &= !csr::MSTATUS_SPP;
            }
            sstatus &= !csr::MSTATUS_SIE;
            self.csrs.sstatus = sstatus;

            let sstatus_mask = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
            self.csrs.mstatus = (self.csrs.mstatus & !sstatus_mask) | (sstatus & sstatus_mask);

            self.privilege = PrivilegeMode::Supervisor;
            let stvec_base = self.csrs.stvec & !3;
            let trap_handler_pc = stvec_base
                + (if (self.csrs.stvec & 1) != 0 && is_interrupt { 4 * code } else { 0 });

            self.pc = trap_handler_pc;
        } else {
            self.csrs.mcause = if is_interrupt { CAUSE_INTERRUPT_BIT | code } else { code };
            self.csrs.mepc = epc;
            self.csrs.mtval = tval;

            let mut mstatus = self.csrs.mstatus;
            if (mstatus & csr::MSTATUS_MIE) != 0 {
                mstatus |= csr::MSTATUS_MPIE;
            } else {
                mstatus &= !csr::MSTATUS_MPIE;
            }
            mstatus &= !csr::MSTATUS_MPP;
            mstatus |= (self.privilege.to_u8() as u64) << csr::MSTATUS_MPP_SHIFT;
            mstatus &= !csr::MSTATUS_MIE;
            self.csrs.mstatus = mstatus;

            self.privilege = PrivilegeMode::Machine;
            let mtvec_base = self.csrs.mtvec & !3;
            let target_pc = mtvec_base
                + (if (self.csrs.mtvec & 1) != 0 && is_interrupt { 4 * code } else { 0 });
            self.pc = target_pc;
        }

        self.stats.traps_taken += 1;
    }

    /// Executes the `MRET` instruction (Return from Machine Mode).
    pub(crate) const fn do_mret(&mut self) {
        self.clear_reservation(); // MRET invalidates reservations
        self.pc = self.csrs.mepc & !1;
        let mstatus = self.csrs.mstatus;
        let mpp = (mstatus >> csr::MSTATUS_MPP_SHIFT) & csr::MSTATUS_MPP_MASK;
        let mpie = (mstatus & csr::MSTATUS_MPIE) != 0;

        self.privilege = PrivilegeMode::from_u8(mpp as u8);
        let mut new_mstatus = mstatus;
        if mpie {
            new_mstatus |= csr::MSTATUS_MIE;
        } else {
            new_mstatus &= !csr::MSTATUS_MIE;
        }
        new_mstatus |= csr::MSTATUS_MPIE;
        new_mstatus &= !csr::MSTATUS_MPP;
        // Per spec 3.1.6.1: if xPP != M, xRET also sets MPRV=0
        if mpp != PrivilegeMode::Machine.to_u8() as u64 {
            new_mstatus &= !csr::MSTATUS_MPRV;
        }

        self.csrs.mstatus = new_mstatus;
    }

    /// Executes the `SRET` instruction (Return from Supervisor Mode).
    pub(crate) const fn do_sret(&mut self) {
        self.clear_reservation(); // SRET invalidates reservations
        self.pc = self.csrs.sepc & !1;
        let sstatus = self.csrs.sstatus;
        let spp = (sstatus & csr::MSTATUS_SPP) != 0;
        let spie = (sstatus & csr::MSTATUS_SPIE) != 0;

        self.privilege = if spp { PrivilegeMode::Supervisor } else { PrivilegeMode::User };
        let mut new_sstatus = sstatus;
        if spie {
            new_sstatus |= csr::MSTATUS_SIE;
        } else {
            new_sstatus &= !csr::MSTATUS_SIE;
        }
        new_sstatus |= csr::MSTATUS_SPIE;
        new_sstatus &= !csr::MSTATUS_SPP;

        self.csrs.sstatus = new_sstatus;
        let mask = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
        let mut new_mstatus = (self.csrs.mstatus & !mask) | (new_sstatus & mask);
        // Per spec 3.1.6.1: SRET returns to S or U (never M), so always clear MPRV
        new_mstatus &= !csr::MSTATUS_MPRV;
        self.csrs.mstatus = new_mstatus;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::soc::builder::System;

    #[test]
    fn test_trap_direct_mode_ecall() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.regs.write(abi::REG_A7, sys_ops::SYS_EXIT);
        cpu.regs.write(abi::REG_A0, 42);

        cpu.trap(&Trap::EnvironmentCallFromMMode, 0x1000);
        assert_eq!(cpu.exit_code, Some(42));
    }

    #[test]
    fn test_trap_direct_mode_illegal_instruction() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.trap(&Trap::IllegalInstruction(0), 0x1000);
        assert_eq!(cpu.exit_code, Some(0));
    }

    #[test]
    fn test_do_mret() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csrs.mepc = 0x2000;
        cpu.csrs.mstatus = (PrivilegeMode::Supervisor.to_u8() as u64) << csr::MSTATUS_MPP_SHIFT;
        cpu.csrs.mstatus |= csr::MSTATUS_MPIE;

        cpu.do_mret();

        assert_eq!(cpu.pc, 0x2000);
        assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
        assert_eq!(cpu.csrs.mstatus & csr::MSTATUS_MIE, csr::MSTATUS_MIE);
    }

    #[test]
    fn test_do_sret() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csrs.sepc = 0x3000;
        cpu.csrs.sstatus = csr::MSTATUS_SPP | csr::MSTATUS_SPIE;

        cpu.do_sret();

        assert_eq!(cpu.pc, 0x3000);
        assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
        assert_eq!(cpu.csrs.sstatus & csr::MSTATUS_SIE, csr::MSTATUS_SIE);
    }
}

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
use crate::isa::privileged::cause::{exception, interrupt};

impl Cpu {
    /// Handles a trap (exception or interrupt).
    ///
    /// # Arguments
    ///
    /// * `cause` - The type of trap that occurred.
    /// * `epc` - The Exception Program Counter (PC where the trap occurred).
    pub fn trap(&mut self, cause: Trap, epc: u64) {
        self.load_reservation = None;

        if self.direct_mode {
            if !matches!(cause, Trap::EnvironmentCallFromUMode) {
                if matches!(cause, Trap::IllegalInstruction(0)) {
                    self.exit_code = Some(0);
                    return;
                }
                eprintln!(
                    "\n[!] Fatal trap in direct mode: {:?} at PC {:#x}",
                    cause, epc
                );
                self.exit_code = Some(1);
                return;
            }
        }

        let is_timer = matches!(
            cause,
            Trap::MachineTimerInterrupt | Trap::SupervisorTimerInterrupt
        );
        let is_ecall = matches!(
            cause,
            Trap::EnvironmentCallFromUMode
                | Trap::EnvironmentCallFromSMode
                | Trap::EnvironmentCallFromMMode
        );

        if self.trace {
            if self.csrs.stvec == 0x80000530 || epc == 0x80000530 {
                println!(
                    "[Trap] Cause: {:?} | EPC: {:#x} | Priv: {} | STVEC: {:#x}",
                    cause, epc, self.privilege, self.csrs.stvec
                );
            } else if !is_timer && !is_ecall {
                println!(
                    "[Trap] Cause: {:?} | EPC: {:#x} | Priv: {}",
                    cause, epc, self.privilege
                );
            }
        }

        let (is_interrupt, code) = match cause {
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

        let deleg_mask = if is_interrupt {
            self.csrs.mideleg
        } else {
            self.csrs.medeleg
        };
        let mut delegate_to_s =
            (self.privilege <= PrivilegeMode::Supervisor) && ((deleg_mask >> code) & 1) != 0;

        if self.privilege == PrivilegeMode::User
            && !is_interrupt
            && !delegate_to_s
            && (self.csrs.stvec & !3) != 0
        {
            if !is_ecall {
                eprintln!(
                    "[TRAP DEBUG] User mode trap not delegated but STVEC is set. Forcing delegation. Cause={:?} Code={} MEDELEG={:#x} STVEC={:#x}",
                    cause, code, self.csrs.medeleg, self.csrs.stvec
                );
            }
            delegate_to_s = true;
        }

        if delegate_to_s {
            let stvec_base = self.csrs.stvec & !3;
            let trap_handler_pc = stvec_base
                + (if (self.csrs.stvec & 1) != 0 && is_interrupt {
                    4 * code
                } else {
                    0
                });

            if epc == trap_handler_pc {
                let fault = Trap::DoubleFault(epc);
                eprintln!(
                    "[FATAL] {} detected! CPU faulted at S-mode trap handler.",
                    fault
                );
                self.exit_code = Some(102);
                return;
            }
        } else {
            let mtvec_base = self.csrs.mtvec & !3;
            let trap_handler_pc = mtvec_base
                + (if (self.csrs.mtvec & 1) != 0 && is_interrupt {
                    4 * code
                } else {
                    0
                });

            if epc == trap_handler_pc {
                let fault = Trap::DoubleFault(epc);
                eprintln!(
                    "[FATAL] {} detected! CPU faulted at M-mode trap handler.",
                    fault
                );
                self.exit_code = Some(102);
                return;
            }
        }

        let tval = match cause {
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
            self.csrs.scause = if is_interrupt {
                CAUSE_INTERRUPT_BIT | code
            } else {
                code
            };

            // Implementation Note: Kernel Relocation for SEPC
            //
            // When the MMU is enabled (SATP mode != Bare), the kernel is typically linked
            // at a high virtual address (0xffffffff80000000) but loaded at a physical address
            // (0x80200000). During early boot, before paging is fully initialized, PC values
            // may be physical addresses.
            //
            // If a trap occurs during this transition (EPC points to physical kernel address),
            // we need to adjust SEPC to the corresponding virtual address so that SRET returns
            // to the correct location in the virtual address space.
            //
            // This adjustment applies when:
            // 1. MMU is enabled (SATP mode != 0)
            // 2. EPC is in kernel physical range [0x80200000, 0x82200000)
            // 3. EPC is below the kernel virtual base (not already relocated)
            //
            // The relocation offset is: KERNEL_VIRT_BASE - KERNEL_PHYS_BASE
            // This matches the linker script offset for typical RISC-V kernels.
            let mut sepc_value = epc;
            let mmu_enabled = (self.csrs.satp >> 60) != 0;
            if mmu_enabled {
                const KERNEL_PHYS_BASE: u64 = 0x80200000;
                const KERNEL_VIRT_BASE: u64 = 0xffffffff80000000;
                const RELOC_OFFSET: u64 = KERNEL_VIRT_BASE.wrapping_sub(KERNEL_PHYS_BASE);

                if epc >= KERNEL_PHYS_BASE
                    && epc < KERNEL_PHYS_BASE + 0x2000000
                    && epc < KERNEL_VIRT_BASE
                {
                    sepc_value = epc.wrapping_add(RELOC_OFFSET);
                }
            }

            self.csrs.sepc = sepc_value;
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
                + (if (self.csrs.stvec & 1) != 0 && is_interrupt {
                    4 * code
                } else {
                    0
                });

            self.pc = trap_handler_pc;
        } else {
            self.csrs.mcause = if is_interrupt {
                CAUSE_INTERRUPT_BIT | code
            } else {
                code
            };
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

            let target_pc = if mtvec_base == 0 {
                eprintln!(
                    "[WARNING] Trap to machine mode but MTVEC is 0! This indicates missing trap handler setup."
                );
                let stvec_base = self.csrs.stvec & !3;
                if stvec_base != 0 {
                    stvec_base
                } else {
                    mtvec_base
                }
            } else {
                mtvec_base
                    + (if (self.csrs.mtvec & 1) != 0 && is_interrupt {
                        4 * code
                    } else {
                        0
                    })
            };
            self.pc = target_pc;
        }

        self.stats.traps_taken += 1;
        self.if_id = Default::default();
        self.id_ex = Default::default();
        self.ex_mem = Default::default();
        self.mem_wb = Default::default();
    }

    /// Executes the `MRET` instruction (Return from Machine Mode).
    pub(crate) fn do_mret(&mut self) {
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

        self.csrs.mstatus = new_mstatus;
        self.if_id = Default::default();
        self.id_ex = Default::default();
    }

    /// Executes the `SRET` instruction (Return from Supervisor Mode).
    pub(crate) fn do_sret(&mut self) {
        let mut sepc = self.csrs.sepc & !1;
        let mmu_enabled = (self.csrs.satp >> 60) != 0;
        if mmu_enabled {
            const KERNEL_PHYS_BASE: u64 = 0x80200000;
            const KERNEL_VIRT_BASE: u64 = 0xffffffff80000000;
            const RELOC_OFFSET: u64 = KERNEL_VIRT_BASE.wrapping_sub(KERNEL_PHYS_BASE);

            if sepc >= KERNEL_PHYS_BASE
                && sepc < KERNEL_PHYS_BASE + 0x2000000
                && sepc < KERNEL_VIRT_BASE
            {
                sepc = sepc.wrapping_add(RELOC_OFFSET);
            }
        }

        self.pc = sepc;
        let sstatus = self.csrs.sstatus;
        let spp = (sstatus & csr::MSTATUS_SPP) != 0;
        let spie = (sstatus & csr::MSTATUS_SPIE) != 0;

        self.privilege = if spp {
            PrivilegeMode::Supervisor
        } else {
            PrivilegeMode::User
        };
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
        self.csrs.mstatus = (self.csrs.mstatus & !mask) | (new_sstatus & mask);

        self.if_id = Default::default();
        self.id_ex = Default::default();
    }
}

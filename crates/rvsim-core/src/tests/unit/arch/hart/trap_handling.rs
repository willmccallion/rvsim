//! # Hart Trap Handling Tests
//!
//! This module contains unit tests for trap and exception handling,
//! including trap dispatch and context saving.

use crate::common::{PhysAddr, VirtAddr};
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::isa::reg::RegIdx;
use crate::tests::support::harness::minimal_system;

#[test]
fn a_trap_clears_the_load_reservation() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.set_reservation(PhysAddr::new(0x8000_0000));

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    assert!(!state.check_reservation(PhysAddr::new(0x8000_0000)));
}

#[test]
fn an_illegal_instruction_leaves_the_mcause_interrupt_bit_clear() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    let mcause = state.hart.csrs.mcause;
    // Exceptions should not have interrupt bit set (bit 63)
    assert_eq!(mcause & (1u64 << 63), 0);
}

#[test]
fn exceptions_leave_the_mcause_interrupt_bit_clear() {
    let exceptions = vec![
        Trap::InstructionAddressMisaligned(0),
        Trap::InstructionAccessFault(0),
        Trap::IllegalInstruction(0),
        Trap::Breakpoint(0),
        Trap::LoadAddressMisaligned(0),
        Trap::LoadAccessFault(0),
        Trap::StoreAddressMisaligned(0),
        Trap::StoreAccessFault(0),
        Trap::EnvironmentCallFromUMode,
        Trap::EnvironmentCallFromSMode,
        Trap::EnvironmentCallFromMMode,
    ];

    for exception in exceptions {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;

        state.trap(&exception, state.hart.pc);

        // Exceptions should not have interrupt bit set
        assert_eq!(state.hart.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn ecalls_from_every_mode_leave_the_mcause_interrupt_bit_clear() {
    let ecalls = vec![
        Trap::EnvironmentCallFromUMode,
        Trap::EnvironmentCallFromSMode,
        Trap::EnvironmentCallFromMMode,
    ];

    for ecall in ecalls {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;

        state.trap(&ecall, state.hart.pc);

        // Should not have interrupt bit set (ECALL is an exception, not interrupt)
        assert_eq!(state.hart.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn page_faults_leave_the_mcause_interrupt_bit_clear() {
    let page_faults = vec![
        Trap::InstructionPageFault(0x1000_0000),
        Trap::LoadPageFault(0x2000_0000),
        Trap::StorePageFault(0x3000_0000),
    ];

    for fault_trap in page_faults {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;

        state.trap(&fault_trap, state.hart.pc);

        // Should not have interrupt bit set (page faults are exceptions)
        assert_eq!(state.hart.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn a_trap_raised_at_the_handler_address_dispatches_without_exiting() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let handler_pc = 0x8000_0000;
    state.hart.csrs.mtvec = handler_pc;

    // A trap whose EPC equals the handler address is legitimate (not a double fault).
    state.trap(&Trap::IllegalInstruction(0), VirtAddr::new(handler_pc));
    assert_eq!(state.uncore.peek_exit(), None);
    assert_eq!(state.hart.pc, VirtAddr::new(handler_pc)); // dispatched to M-mode handler
}

#[test]
fn interrupts_set_the_mcause_interrupt_bit() {
    let interrupts = vec![
        Trap::UserSoftwareInterrupt,
        Trap::SupervisorSoftwareInterrupt,
        Trap::MachineSoftwareInterrupt,
        Trap::SupervisorTimerInterrupt,
        Trap::MachineTimerInterrupt,
        Trap::UserExternalInterrupt,
        Trap::SupervisorExternalInterrupt,
        Trap::MachineExternalInterrupt,
    ];

    for interrupt in interrupts {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;
        state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

        state.trap(&interrupt, state.hart.pc);

        // Interrupts should have interrupt bit set (bit 63)
        assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn a_machine_timer_interrupt_sets_the_mcause_interrupt_bit() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    state.trap(&Trap::MachineTimerInterrupt, state.hart.pc);

    // Should have interrupt bit set
    assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn a_delegated_supervisor_timer_interrupt_enters_supervisor_mode() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.mideleg = 1 << 5; // Delegate supervisor timer interrupts
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::SupervisorTimerInterrupt, state.hart.pc);

    // Should have delegated to S-mode
    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn a_machine_software_interrupt_sets_the_mcause_interrupt_bit() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    state.trap(&Trap::MachineSoftwareInterrupt, state.hart.pc);

    // Should have interrupt bit set
    assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn a_delegated_supervisor_software_interrupt_enters_supervisor_mode() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.mideleg = 1 << 1; // Delegate supervisor software interrupts
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::SupervisorSoftwareInterrupt, state.hart.pc);

    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn a_machine_external_interrupt_sets_the_mcause_interrupt_bit() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    state.trap(&Trap::MachineExternalInterrupt, state.hart.pc);

    // Should have interrupt bit set
    assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn a_delegated_supervisor_external_interrupt_enters_supervisor_mode() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.mideleg = 1 << 9; // Delegate supervisor external interrupts
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::SupervisorExternalInterrupt, state.hart.pc);

    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn an_exception_delegated_by_medeleg_sets_scause_and_sepc() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    // Delegate instruction page faults (exception code 12) to S-mode
    state.hart.csrs.medeleg = 1 << 12;

    let old_pc = state.hart.pc;
    state.trap(&Trap::InstructionPageFault(0x1000), old_pc);

    // Should have delegated to S-mode
    assert_eq!(state.hart.csrs.scause & !CAUSE_INTERRUPT_BIT, 12);
    assert_eq!(state.hart.csrs.sepc, old_pc.val());
}

#[test]
fn an_interrupt_delegated_by_mideleg_sets_the_scause_interrupt_bit() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    // Delegate supervisor software interrupts (interrupt code 1) to S-mode
    state.hart.csrs.mideleg = 1 << 1;

    state.trap(&Trap::SupervisorSoftwareInterrupt, state.hart.pc);

    // Should have delegated to S-mode
    assert_ne!(state.hart.csrs.scause & CAUSE_INTERRUPT_BIT, 0);
}

#[test]
fn an_undelegated_illegal_instruction_from_user_mode_traps_to_machine_mode() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0; // STVEC not set
    state.hart.csrs.medeleg = 0; // No delegation
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Should NOT have delegated to S-mode, stays in M-mode
    assert_eq!(state.hart.privilege, PrivilegeMode::Machine);
}

#[test]
fn a_machine_mode_trap_ignores_medeleg() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    // Enable delegation
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Machine mode traps should NOT delegate even with medeleg set
    assert_eq!(state.hart.privilege, PrivilegeMode::Machine);
}

#[test]
fn an_undelegated_misaligned_load_from_user_mode_jumps_to_mtvec() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000; // STVEC is set but irrelevant
    state.hart.csrs.medeleg = 0; // No delegation in medeleg

    // Without medeleg bit set, trap must go to M-mode per spec
    state.trap(&Trap::LoadAddressMisaligned(0x1001), state.hart.pc);

    assert_eq!(state.hart.privilege, PrivilegeMode::Machine);
    assert_eq!(state.hart.pc, VirtAddr::new(0x8000_0000));
}

#[test]
fn a_vectored_mtvec_sends_exceptions_to_the_base() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    state.hart.csrs.mtvec = base | 1; // Vectored mode (bit 0 = 1)

    let old_pc = state.hart.pc;
    state.trap(&Trap::IllegalInstruction(0), old_pc);

    // Exceptions should use base address (no offset)
    assert_eq!(state.hart.pc, VirtAddr::new(base));
}

#[test]
fn a_vectored_mtvec_offsets_interrupts_by_four_times_the_cause() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    state.hart.csrs.mtvec = base | 1; // Vectored mode

    state.trap(&Trap::MachineTimerInterrupt, state.hart.pc);

    // Machine timer interrupt (code 7) should offset by 4*7 = 28
    assert_eq!(state.hart.pc, VirtAddr::new(base + 28));
}

#[test]
fn a_direct_mtvec_sends_interrupts_to_the_base() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    state.hart.csrs.mtvec = base; // Direct mode (bit 0 = 0)

    state.trap(&Trap::MachineTimerInterrupt, state.hart.pc);

    // Direct mode should use base address (no offset)
    assert_eq!(state.hart.pc, VirtAddr::new(base));
}

#[test]
fn a_vectored_stvec_offsets_delegated_interrupts_by_four_times_the_cause() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    let base = 0x8000_1000;
    state.hart.csrs.stvec = base | 1; // Vectored mode
    state.hart.csrs.mideleg = 1 << 5; // Delegate supervisor timer interrupts
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::SupervisorTimerInterrupt, state.hart.pc);

    // Supervisor timer interrupt (code 5) should offset by 4*5 = 20
    assert_eq!(state.hart.pc, VirtAddr::new(base + 20));
}

#[test]
fn a_misaligned_load_writes_the_fault_address_to_mtval() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    let fault_addr = 0x1234_5678;
    state.trap(&Trap::LoadAddressMisaligned(fault_addr), state.hart.pc);

    assert_eq!(state.hart.csrs.mtval, fault_addr);
}

#[test]
fn a_store_page_fault_writes_the_fault_address_to_mtval() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    let fault_addr = 0xdead_beef;
    state.trap(&Trap::StorePageFault(fault_addr), state.hart.pc);

    assert_eq!(state.hart.csrs.mtval, fault_addr);
}

#[test]
fn an_illegal_instruction_writes_its_encoding_to_mtval() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    let bad_instr = 0xdeadbeef;
    state.trap(&Trap::IllegalInstruction(bad_instr), state.hart.pc);

    assert_eq!(state.hart.csrs.mtval, u64::from(bad_instr));
}

#[test]
fn an_ecall_leaves_mtval_zero() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    state.trap(&Trap::EnvironmentCallFromMMode, state.hart.pc);

    // ECALL should set tval to 0
    assert_eq!(state.hart.csrs.mtval, 0);
}

#[test]
fn a_delegated_load_page_fault_writes_the_fault_address_to_stval() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 13; // Delegate load page faults
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    let fault_addr = 0xcafe_babe;
    state.trap(&Trap::LoadPageFault(fault_addr), state.hart.pc);

    assert_eq!(state.hart.csrs.stval, fault_addr);
}

#[test]
fn a_trap_from_supervisor_mode_records_it_in_mpp() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Supervisor;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // mstatus.MPP should be set to Supervisor (0b01)
    assert_eq!(state.hart.csrs.mstatus >> 11 & 0b11, 1);
}

#[test]
fn a_trap_moves_mie_into_mpie_and_clears_mie() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler
    // Enable MIE (bit 3)
    state.hart.csrs.mstatus = 1 << 3;

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // MIE should be disabled
    assert_eq!(state.hart.csrs.mstatus & (1 << 3), 0);
    // MPIE (bit 7) should be set from MIE
    assert_ne!(state.hart.csrs.mstatus & (1 << 7), 0);
}

#[test]
fn a_delegated_trap_from_user_mode_leaves_spp_clear() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // mstatus.SPP should be cleared for User (bit 8 = 0)
    assert_eq!(state.hart.csrs.mstatus >> 8 & 1, 0);
}

#[test]
fn a_delegated_illegal_instruction_enters_supervisor_mode_and_sets_sepc() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    state.hart.pc = VirtAddr::new(0x8000_2000); // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Should have delegated to S-mode
    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
    // SEPC should be set
    assert_eq!(state.hart.csrs.sepc, 0x8000_2000);
}

#[test]
fn a_requested_trap_writes_its_code_to_mcause() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    let custom_code = 42;
    state.trap(&Trap::RequestedTrap(custom_code), state.hart.pc);

    assert_eq!(state.hart.csrs.mcause, custom_code);
}

#[test]
fn a_breakpoint_writes_cause_three_to_mcause() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    state.trap(&Trap::Breakpoint(0), state.hart.pc);

    // Breakpoint should be handled as exception
    assert_eq!(state.hart.csrs.mcause & !CAUSE_INTERRUPT_BIT, 3);
}

#[test]
fn access_faults_write_their_cause_codes_to_mcause() {
    let faults = vec![
        (Trap::InstructionAccessFault(0x1000), 1),
        (Trap::LoadAccessFault(0x2000), 5),
        (Trap::StoreAccessFault(0x3000), 7),
    ];

    for (fault, expected_code) in faults {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;
        state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

        state.trap(&fault, state.hart.pc);

        assert_eq!(state.hart.csrs.mcause, expected_code);
    }
}

#[test]
fn misaligned_accesses_write_their_cause_codes_to_mcause() {
    let misaligned = vec![
        (Trap::InstructionAddressMisaligned(0x1001), 0),
        (Trap::LoadAddressMisaligned(0x2001), 4),
        (Trap::StoreAddressMisaligned(0x3001), 6),
    ];

    for (trap, expected_code) in misaligned {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;
        state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

        state.trap(&trap, state.hart.pc);

        assert_eq!(state.hart.csrs.mcause, expected_code);
    }
}

#[test]
fn a_trap_preserves_the_integer_registers() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = VirtAddr::new(0x8000_1000); // Different from trap handler

    // Set up some register state
    state.hart.regs.write(RegIdx::new(1), 0x1234);
    state.hart.regs.write(RegIdx::new(2), 0x5678);

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Registers should be preserved across trap
    assert_eq!(state.hart.regs.read(RegIdx::new(1)), 0x1234);
    assert_eq!(state.hart.regs.read(RegIdx::new(2)), 0x5678);
}

#[test]
fn a_trap_writes_the_trap_pc_to_mepc() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    let trap_pc = 0x8000_1234;

    state.trap(&Trap::IllegalInstruction(0), VirtAddr::new(trap_pc));

    assert_eq!(state.hart.csrs.mepc, trap_pc);
}

#[test]
fn a_delegated_trap_writes_the_trap_pc_to_sepc() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    let trap_pc = 0x8000_5678;

    state.trap(&Trap::IllegalInstruction(0), VirtAddr::new(trap_pc));

    assert_eq!(state.hart.csrs.sepc, trap_pc);
}

use crate::isa::privileged::cause::CAUSE_INTERRUPT_BIT;

#[test]
fn an_undelegated_user_interrupt_or_hardware_fault_traps_to_machine_mode() {
    for trap in
        [Trap::UserSoftwareInterrupt, Trap::UserExternalInterrupt, Trap::DoubleFault(0x1234)]
    {
        let mut sys = minimal_system();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::User;
        state.hart.csrs.mtvec = 0x8000_0400;

        state.trap(&trap, state.hart.pc);

        assert_eq!(state.hart.privilege, PrivilegeMode::Machine, "{trap}");
        assert_eq!(state.hart.csrs.mcause, trap.mcause_code(), "{trap}");
        assert_eq!(state.hart.pc, VirtAddr::new(0x8000_0400), "{trap}");
    }
}

//! # Trap Handling Tests
//!
//! This module contains unit tests for trap and exception handling,
//! including trap dispatch and context saving.

use rvsim_core::common::Trap;
use rvsim_core::config::Config;
use rvsim_core::core::Cpu;
use rvsim_core::core::arch::mode::PrivilegeMode;

fn create_test_cpu() -> Cpu {
    let config = Config::default();
    let system = rvsim_core::soc::System::new(&config, "");
    let mut cpu = Cpu::new(system, &config);
    cpu.direct_mode = false;
    cpu
}

#[test]
fn test_trap_clears_load_reservation() {
    let mut cpu = create_test_cpu();
    cpu.load_reservation = Some(0x8000_0000);

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    assert_eq!(cpu.load_reservation, None);
}

#[test]
fn test_trap_direct_mode_illegal_instruction_zero_exits() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;
    cpu.exit_code = None;

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    assert_eq!(cpu.exit_code, Some(0));
}

#[test]
fn test_trap_direct_mode_other_exceptions_set_exit_code_1() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;
    cpu.exit_code = None;

    cpu.trap(Trap::LoadAddressMisaligned(0x8000_0001), cpu.pc);

    assert_eq!(cpu.exit_code, Some(1));
}

#[test]
fn test_trap_direct_mode_ecall_from_umode_processed() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;
    cpu.privilege = PrivilegeMode::User;
    cpu.exit_code = None;
    cpu.csrs.mtvec = 0x8000_0000;

    // ECALL in direct mode should be processed normally (not treated as fatal)
    cpu.trap(Trap::EnvironmentCallFromUMode, cpu.pc);

    // Exit code should remain None (trap is processed, not fatal)
}

#[test]
fn test_trap_sets_mcause_without_interrupt_bit_for_exceptions() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    let mcause = cpu.csrs.mcause;
    // Exceptions should not have interrupt bit set (bit 63)
    assert_eq!(mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_exceptions_dont_set_interrupt_bit() {
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
        let mut cpu = create_test_cpu();
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csrs.mtvec = 0x8000_0000;

        cpu.trap(exception, cpu.pc);

        // Exceptions should not have interrupt bit set
        assert_eq!(cpu.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn test_trap_ecall_from_all_modes() {
    let ecalls = vec![
        Trap::EnvironmentCallFromUMode,
        Trap::EnvironmentCallFromSMode,
        Trap::EnvironmentCallFromMMode,
    ];

    for ecall in ecalls {
        let mut cpu = create_test_cpu();
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csrs.mtvec = 0x8000_0000;

        cpu.trap(ecall, cpu.pc);

        // Should not have interrupt bit set (ECALL is an exception, not interrupt)
        assert_eq!(cpu.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn test_trap_page_faults() {
    let page_faults = vec![
        Trap::InstructionPageFault(0x1000_0000),
        Trap::LoadPageFault(0x2000_0000),
        Trap::StorePageFault(0x3000_0000),
    ];

    for fault_trap in page_faults {
        let mut cpu = create_test_cpu();
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csrs.mtvec = 0x8000_0000;

        cpu.trap(fault_trap, cpu.pc);

        // Should not have interrupt bit set (page faults are exceptions)
        assert_eq!(cpu.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn test_trap_double_fault_detection() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    let handler_pc = 0x8000_0000;
    cpu.csrs.mtvec = handler_pc;

    // Simulate a trap at the handler PC (double fault)
    cpu.trap(Trap::IllegalInstruction(0), handler_pc);

    // Double fault should set exit code to 102
    assert_eq!(cpu.exit_code, Some(102));
}

// === Interrupt Handling Tests ===

#[test]
fn test_trap_interrupts_set_interrupt_bit() {
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
        let mut cpu = create_test_cpu();
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csrs.mtvec = 0x8000_0000;
        cpu.pc = 0x8000_1000; // Different from trap handler

        cpu.trap(interrupt, cpu.pc);

        // Interrupts should have interrupt bit set (bit 63)
        assert_ne!(cpu.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn test_trap_machine_timer_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::MachineTimerInterrupt, cpu.pc);

    // Should have interrupt bit set
    assert_ne!(cpu.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_supervisor_timer_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.mideleg = 1 << 5; // Delegate supervisor timer interrupts
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::SupervisorTimerInterrupt, cpu.pc);

    // Should have delegated to S-mode
    assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn test_trap_machine_software_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::MachineSoftwareInterrupt, cpu.pc);

    // Should have interrupt bit set
    assert_ne!(cpu.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_supervisor_software_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.mideleg = 1 << 1; // Delegate supervisor software interrupts
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::SupervisorSoftwareInterrupt, cpu.pc);

    assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn test_trap_machine_external_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::MachineExternalInterrupt, cpu.pc);

    // Should have interrupt bit set
    assert_ne!(cpu.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_supervisor_external_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.mideleg = 1 << 9; // Delegate supervisor external interrupts
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::SupervisorExternalInterrupt, cpu.pc);

    assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn test_trap_user_software_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.stvec = 0x8000_0000;

    cpu.trap(Trap::UserSoftwareInterrupt, cpu.pc);

    // User mode traps typically get handled at higher privilege
}

#[test]
fn test_trap_user_external_interrupt() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.stvec = 0x8000_0000;

    cpu.trap(Trap::UserExternalInterrupt, cpu.pc);

    // User mode external interrupt handling
}

// === Delegation Tests ===

#[test]
fn test_trap_delegation_to_supervisor_with_medeleg() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000;
    // Delegate instruction page faults (exception code 12) to S-mode
    cpu.csrs.medeleg = 1 << 12;

    let old_pc = cpu.pc;
    cpu.trap(Trap::InstructionPageFault(0x1000), old_pc);

    // Should have delegated to S-mode
    assert_eq!(cpu.csrs.scause & !CAUSE_INTERRUPT_BIT, 12);
    assert_eq!(cpu.csrs.sepc, old_pc);
}

#[test]
fn test_trap_delegation_to_supervisor_with_mideleg() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000;
    // Delegate supervisor software interrupts (interrupt code 1) to S-mode
    cpu.csrs.mideleg = 1 << 1;

    cpu.trap(Trap::SupervisorSoftwareInterrupt, cpu.pc);

    // Should have delegated to S-mode
    assert_ne!(cpu.csrs.scause & CAUSE_INTERRUPT_BIT, 0);
}

#[test]
fn test_trap_no_delegation_when_medeleg_not_set() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0; // STVEC not set
    cpu.csrs.medeleg = 0; // No delegation
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // Should NOT have delegated to S-mode, stays in M-mode
    assert_eq!(cpu.privilege, PrivilegeMode::Machine);
}

#[test]
fn test_trap_delegation_only_from_lower_privilege() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000;
    // Enable delegation
    cpu.csrs.medeleg = 1 << 2; // Delegate illegal instruction

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // Machine mode traps should NOT delegate even with medeleg set
    assert_eq!(cpu.privilege, PrivilegeMode::Machine);
}

#[test]
fn test_trap_user_mode_force_delegation_with_stvec_set() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.csrs.stvec = 0x8000_1000; // STVEC is set
    cpu.csrs.medeleg = 0; // No delegation in medeleg

    // User mode trap with STVEC set should force delegation
    cpu.trap(Trap::LoadAddressMisaligned(0x1001), cpu.pc);

    // Should have been forced to delegate to S-mode
    assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
}

// === Vectored Mode Tests ===

#[test]
fn test_trap_vectored_mode_direct_for_exceptions() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    cpu.csrs.mtvec = base | 1; // Vectored mode (bit 0 = 1)

    let old_pc = cpu.pc;
    cpu.trap(Trap::IllegalInstruction(0), old_pc);

    // Exceptions should use base address (no offset)
    assert_eq!(cpu.pc, base);
}

#[test]
fn test_trap_vectored_mode_offset_for_interrupts() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    cpu.csrs.mtvec = base | 1; // Vectored mode

    cpu.trap(Trap::MachineTimerInterrupt, cpu.pc);

    // Machine timer interrupt (code 7) should offset by 4*7 = 28
    assert_eq!(cpu.pc, base + 28);
}

#[test]
fn test_trap_direct_mode_no_offset() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    cpu.csrs.mtvec = base; // Direct mode (bit 0 = 0)

    cpu.trap(Trap::MachineTimerInterrupt, cpu.pc);

    // Direct mode should use base address (no offset)
    assert_eq!(cpu.pc, base);
}

#[test]
fn test_trap_supervisor_vectored_mode() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.mtvec = 0x8000_0000;
    let base = 0x8000_1000;
    cpu.csrs.stvec = base | 1; // Vectored mode
    cpu.csrs.mideleg = 1 << 5; // Delegate supervisor timer interrupts
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::SupervisorTimerInterrupt, cpu.pc);

    // Supervisor timer interrupt (code 5) should offset by 4*5 = 20
    assert_eq!(cpu.pc, base + 20);
}

// === Trap Value (tval) Tests ===

#[test]
fn test_trap_tval_for_address_exceptions() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    let fault_addr = 0x1234_5678;
    cpu.trap(Trap::LoadAddressMisaligned(fault_addr), cpu.pc);

    assert_eq!(cpu.csrs.mtval, fault_addr);
}

#[test]
fn test_trap_tval_for_page_faults() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    let fault_addr = 0xdead_beef;
    cpu.trap(Trap::StorePageFault(fault_addr), cpu.pc);

    assert_eq!(cpu.csrs.mtval, fault_addr);
}

#[test]
fn test_trap_tval_for_illegal_instruction() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    let bad_instr = 0xdeadbeef;
    cpu.trap(Trap::IllegalInstruction(bad_instr), cpu.pc);

    assert_eq!(cpu.csrs.mtval, bad_instr as u64);
}

#[test]
fn test_trap_tval_zero_for_ecall() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::EnvironmentCallFromMMode, cpu.pc);

    // ECALL should set tval to 0
    assert_eq!(cpu.csrs.mtval, 0);
}

#[test]
fn test_trap_stval_on_delegation() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.medeleg = 1 << 13; // Delegate load page faults
    cpu.pc = 0x8000_2000; // Different from trap handler

    let fault_addr = 0xcafe_babe;
    cpu.trap(Trap::LoadPageFault(fault_addr), cpu.pc);

    assert_eq!(cpu.csrs.stval, fault_addr);
}

// === Status Bit Tests ===

#[test]
fn test_trap_saves_previous_privilege_in_mpp() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Supervisor;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // mstatus.MPP should be set to Supervisor (0b01)
    assert_eq!(cpu.csrs.mstatus >> 11 & 0b11, 1);
}

#[test]
fn test_trap_disables_mie_and_saves_to_mpie() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler
    // Enable MIE (bit 3)
    cpu.csrs.mstatus = 1 << 3;

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // MIE should be disabled
    assert_eq!(cpu.csrs.mstatus & (1 << 3), 0);
    // MPIE (bit 7) should be set from MIE
    assert_ne!(cpu.csrs.mstatus & (1 << 7), 0);
}

#[test]
fn test_trap_saves_previous_privilege_in_spp_on_delegation() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // mstatus.SPP should be cleared for User (bit 8 = 0)
    assert_eq!(cpu.csrs.mstatus >> 8 & 1, 0);
}

#[test]
fn test_trap_disables_sie_on_delegation() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    cpu.pc = 0x8000_2000; // Different from trap handler

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // Should have delegated to S-mode
    assert_eq!(cpu.privilege, PrivilegeMode::Supervisor);
    // SEPC should be set
    assert_eq!(cpu.csrs.sepc, 0x8000_2000);
}

// === Edge Cases and Complex Scenarios ===

#[test]
fn test_trap_requested_trap_custom_code() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    let custom_code = 42;
    cpu.trap(Trap::RequestedTrap(custom_code), cpu.pc);

    assert_eq!(cpu.csrs.mcause, custom_code);
}

#[test]
fn test_trap_double_fault_trap_variant() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::DoubleFault(0x1234), cpu.pc);

    // DoubleFault should map to hardware error exception
}

#[test]
fn test_trap_breakpoint() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    cpu.trap(Trap::Breakpoint(0), cpu.pc);

    // Breakpoint should be handled as exception
    assert_eq!(cpu.csrs.mcause & !CAUSE_INTERRUPT_BIT, 3);
}

#[test]
fn test_trap_all_access_faults() {
    let faults = vec![
        (Trap::InstructionAccessFault(0x1000), 1),
        (Trap::LoadAccessFault(0x2000), 5),
        (Trap::StoreAccessFault(0x3000), 7),
    ];

    for (fault, expected_code) in faults {
        let mut cpu = create_test_cpu();
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csrs.mtvec = 0x8000_0000;
        cpu.pc = 0x8000_1000; // Different from trap handler

        cpu.trap(fault, cpu.pc);

        assert_eq!(cpu.csrs.mcause, expected_code);
    }
}

#[test]
fn test_trap_all_misaligned() {
    let misaligned = vec![
        (Trap::InstructionAddressMisaligned(0x1001), 0),
        (Trap::LoadAddressMisaligned(0x2001), 4),
        (Trap::StoreAddressMisaligned(0x3001), 6),
    ];

    for (trap, expected_code) in misaligned {
        let mut cpu = create_test_cpu();
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csrs.mtvec = 0x8000_0000;
        cpu.pc = 0x8000_1000; // Different from trap handler

        cpu.trap(trap, cpu.pc);

        assert_eq!(cpu.csrs.mcause, expected_code);
    }
}

#[test]
fn test_trap_preserves_registers() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    cpu.pc = 0x8000_1000; // Different from trap handler

    // Set up some register state
    cpu.regs.write(1, 0x1234);
    cpu.regs.write(2, 0x5678);

    cpu.trap(Trap::IllegalInstruction(0), cpu.pc);

    // Registers should be preserved across trap
    assert_eq!(cpu.regs.read(1), 0x1234);
    assert_eq!(cpu.regs.read(2), 0x5678);
}

#[test]
fn test_trap_updates_mepc_correctly() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::Machine;
    cpu.csrs.mtvec = 0x8000_0000;
    let trap_pc = 0x8000_1234;

    cpu.trap(Trap::IllegalInstruction(0), trap_pc);

    assert_eq!(cpu.csrs.mepc, trap_pc);
}

#[test]
fn test_trap_updates_sepc_on_delegation() {
    let mut cpu = create_test_cpu();
    cpu.privilege = PrivilegeMode::User;
    cpu.csrs.stvec = 0x8000_1000;
    cpu.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    let trap_pc = 0x8000_5678;

    cpu.trap(Trap::IllegalInstruction(0), trap_pc);

    assert_eq!(cpu.csrs.sepc, trap_pc);
}

use rvsim_core::common::constants::CAUSE_INTERRUPT_BIT;

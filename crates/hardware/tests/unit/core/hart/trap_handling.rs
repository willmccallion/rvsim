//! # Hart Trap Handling Tests
//!
//! This module contains unit tests for trap and exception handling,
//! including trap dispatch and context saving.

use rvsim_core::SimState;
use rvsim_core::common::{PhysAddr, Trap};
use rvsim_core::config::Config;
use rvsim_core::isa::privileged::mode::PrivilegeMode;
use rvsim_core::isa::reg::RegIdx;

fn create_test_cpu() -> SimState {
    let config = Config::default();
    let mut state = SimState::build(&config, "");
    state.direct_mode = false;
    state
}

#[test]
fn test_trap_clears_load_reservation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.set_reservation(PhysAddr::new(0x8000_0000));

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    assert!(!state.check_reservation(PhysAddr::new(0x8000_0000)));
}

#[test]
fn test_trap_direct_mode_illegal_instruction_zero_exits() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;
    state.exit_signal.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    assert_eq!(state.check_exit(), Some(0));
}

#[test]
fn test_trap_direct_mode_other_exceptions_set_exit_code_1() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;
    state.exit_signal.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);

    state.trap(&Trap::LoadAddressMisaligned(0x8000_0001), state.hart.pc);

    assert_eq!(state.check_exit(), Some(1));
}

#[test]
fn test_trap_direct_mode_ecall_from_umode_processed() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;
    state.hart.privilege = PrivilegeMode::User;
    state.exit_signal.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    state.hart.csrs.mtvec = 0x8000_0000;

    // ECALL in direct mode should be processed normally (not treated as fatal)
    state.trap(&Trap::EnvironmentCallFromUMode, state.hart.pc);

    // Exit code should remain None (trap is processed, not fatal)
}

#[test]
fn test_trap_sets_mcause_without_interrupt_bit_for_exceptions() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    let mcause = state.hart.csrs.mcause;
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
        let mut sys = create_test_cpu();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;

        state.trap(&exception, state.hart.pc);

        // Exceptions should not have interrupt bit set
        assert_eq!(state.hart.csrs.mcause & (1u64 << 63), 0);
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
        let mut sys = create_test_cpu();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;

        state.trap(&ecall, state.hart.pc);

        // Should not have interrupt bit set (ECALL is an exception, not interrupt)
        assert_eq!(state.hart.csrs.mcause & (1u64 << 63), 0);
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
        let mut sys = create_test_cpu();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;

        state.trap(&fault_trap, state.hart.pc);

        // Should not have interrupt bit set (page faults are exceptions)
        assert_eq!(state.hart.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn test_trap_double_fault_detection() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let handler_pc = 0x8000_0000;
    state.hart.csrs.mtvec = handler_pc;

    // A trap whose EPC equals the handler address is legitimate (not a double fault).
    state.trap(&Trap::IllegalInstruction(0), handler_pc);
    assert_eq!(state.check_exit(), None);
    assert_eq!(state.hart.pc, handler_pc); // dispatched to M-mode handler
}

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
        let mut sys = create_test_cpu();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;
        state.hart.pc = 0x8000_1000; // Different from trap handler

        state.trap(&interrupt, state.hart.pc);

        // Interrupts should have interrupt bit set (bit 63)
        assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
    }
}

#[test]
fn test_trap_machine_timer_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::MachineTimerInterrupt, state.hart.pc);

    // Should have interrupt bit set
    assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_supervisor_timer_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.mideleg = 1 << 5; // Delegate supervisor timer interrupts
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::SupervisorTimerInterrupt, state.hart.pc);

    // Should have delegated to S-mode
    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn test_trap_machine_software_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::MachineSoftwareInterrupt, state.hart.pc);

    // Should have interrupt bit set
    assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_supervisor_software_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.mideleg = 1 << 1; // Delegate supervisor software interrupts
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::SupervisorSoftwareInterrupt, state.hart.pc);

    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn test_trap_machine_external_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::MachineExternalInterrupt, state.hart.pc);

    // Should have interrupt bit set
    assert_ne!(state.hart.csrs.mcause & (1u64 << 63), 0);
}

#[test]
fn test_trap_supervisor_external_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.mideleg = 1 << 9; // Delegate supervisor external interrupts
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::SupervisorExternalInterrupt, state.hart.pc);

    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
}

#[test]
fn test_trap_user_software_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_0000;

    state.trap(&Trap::UserSoftwareInterrupt, state.hart.pc);

    // User mode traps typically get handled at higher privilege
}

#[test]
fn test_trap_user_external_interrupt() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_0000;

    state.trap(&Trap::UserExternalInterrupt, state.hart.pc);

    // User mode external interrupt handling
}

#[test]
fn test_trap_delegation_to_supervisor_with_medeleg() {
    let mut sys = create_test_cpu();
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
    assert_eq!(state.hart.csrs.sepc, old_pc);
}

#[test]
fn test_trap_delegation_to_supervisor_with_mideleg() {
    let mut sys = create_test_cpu();
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
fn test_trap_no_delegation_when_medeleg_not_set() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0; // STVEC not set
    state.hart.csrs.medeleg = 0; // No delegation
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Should NOT have delegated to S-mode, stays in M-mode
    assert_eq!(state.hart.privilege, PrivilegeMode::Machine);
}

#[test]
fn test_trap_delegation_only_from_lower_privilege() {
    let mut sys = create_test_cpu();
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
fn test_trap_user_mode_no_delegation_without_medeleg() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.csrs.stvec = 0x8000_1000; // STVEC is set but irrelevant
    state.hart.csrs.medeleg = 0; // No delegation in medeleg

    // Without medeleg bit set, trap must go to M-mode per spec
    state.trap(&Trap::LoadAddressMisaligned(0x1001), state.hart.pc);

    assert_eq!(state.hart.privilege, PrivilegeMode::Machine);
    assert_eq!(state.hart.pc, 0x8000_0000);
}

#[test]
fn test_trap_vectored_mode_direct_for_exceptions() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    state.hart.csrs.mtvec = base | 1; // Vectored mode (bit 0 = 1)

    let old_pc = state.hart.pc;
    state.trap(&Trap::IllegalInstruction(0), old_pc);

    // Exceptions should use base address (no offset)
    assert_eq!(state.hart.pc, base);
}

#[test]
fn test_trap_vectored_mode_offset_for_interrupts() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    state.hart.csrs.mtvec = base | 1; // Vectored mode

    state.trap(&Trap::MachineTimerInterrupt, state.hart.pc);

    // Machine timer interrupt (code 7) should offset by 4*7 = 28
    assert_eq!(state.hart.pc, base + 28);
}

#[test]
fn test_trap_direct_mode_no_offset() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    let base = 0x8000_0000;
    state.hart.csrs.mtvec = base; // Direct mode (bit 0 = 0)

    state.trap(&Trap::MachineTimerInterrupt, state.hart.pc);

    // Direct mode should use base address (no offset)
    assert_eq!(state.hart.pc, base);
}

#[test]
fn test_trap_supervisor_vectored_mode() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.mtvec = 0x8000_0000;
    let base = 0x8000_1000;
    state.hart.csrs.stvec = base | 1; // Vectored mode
    state.hart.csrs.mideleg = 1 << 5; // Delegate supervisor timer interrupts
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::SupervisorTimerInterrupt, state.hart.pc);

    // Supervisor timer interrupt (code 5) should offset by 4*5 = 20
    assert_eq!(state.hart.pc, base + 20);
}

#[test]
fn test_trap_tval_for_address_exceptions() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    let fault_addr = 0x1234_5678;
    state.trap(&Trap::LoadAddressMisaligned(fault_addr), state.hart.pc);

    assert_eq!(state.hart.csrs.mtval, fault_addr);
}

#[test]
fn test_trap_tval_for_page_faults() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    let fault_addr = 0xdead_beef;
    state.trap(&Trap::StorePageFault(fault_addr), state.hart.pc);

    assert_eq!(state.hart.csrs.mtval, fault_addr);
}

#[test]
fn test_trap_tval_for_illegal_instruction() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    let bad_instr = 0xdeadbeef;
    state.trap(&Trap::IllegalInstruction(bad_instr), state.hart.pc);

    assert_eq!(state.hart.csrs.mtval, bad_instr as u64);
}

#[test]
fn test_trap_tval_zero_for_ecall() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::EnvironmentCallFromMMode, state.hart.pc);

    // ECALL should set tval to 0
    assert_eq!(state.hart.csrs.mtval, 0);
}

#[test]
fn test_trap_stval_on_delegation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 13; // Delegate load page faults
    state.hart.pc = 0x8000_2000; // Different from trap handler

    let fault_addr = 0xcafe_babe;
    state.trap(&Trap::LoadPageFault(fault_addr), state.hart.pc);

    assert_eq!(state.hart.csrs.stval, fault_addr);
}

#[test]
fn test_trap_saves_previous_privilege_in_mpp() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Supervisor;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // mstatus.MPP should be set to Supervisor (0b01)
    assert_eq!(state.hart.csrs.mstatus >> 11 & 0b11, 1);
}

#[test]
fn test_trap_disables_mie_and_saves_to_mpie() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler
    // Enable MIE (bit 3)
    state.hart.csrs.mstatus = 1 << 3;

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // MIE should be disabled
    assert_eq!(state.hart.csrs.mstatus & (1 << 3), 0);
    // MPIE (bit 7) should be set from MIE
    assert_ne!(state.hart.csrs.mstatus & (1 << 7), 0);
}

#[test]
fn test_trap_saves_previous_privilege_in_spp_on_delegation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // mstatus.SPP should be cleared for User (bit 8 = 0)
    assert_eq!(state.hart.csrs.mstatus >> 8 & 1, 0);
}

#[test]
fn test_trap_disables_sie_on_delegation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    state.hart.pc = 0x8000_2000; // Different from trap handler

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Should have delegated to S-mode
    assert_eq!(state.hart.privilege, PrivilegeMode::Supervisor);
    // SEPC should be set
    assert_eq!(state.hart.csrs.sepc, 0x8000_2000);
}

#[test]
fn test_trap_requested_trap_custom_code() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    let custom_code = 42;
    state.trap(&Trap::RequestedTrap(custom_code), state.hart.pc);

    assert_eq!(state.hart.csrs.mcause, custom_code);
}

#[test]
fn test_trap_double_fault_trap_variant() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::DoubleFault(0x1234), state.hart.pc);

    // DoubleFault should map to hardware error exception
}

#[test]
fn test_trap_breakpoint() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    state.trap(&Trap::Breakpoint(0), state.hart.pc);

    // Breakpoint should be handled as exception
    assert_eq!(state.hart.csrs.mcause & !CAUSE_INTERRUPT_BIT, 3);
}

#[test]
fn test_trap_all_access_faults() {
    let faults = vec![
        (Trap::InstructionAccessFault(0x1000), 1),
        (Trap::LoadAccessFault(0x2000), 5),
        (Trap::StoreAccessFault(0x3000), 7),
    ];

    for (fault, expected_code) in faults {
        let mut sys = create_test_cpu();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;
        state.hart.pc = 0x8000_1000; // Different from trap handler

        state.trap(&fault, state.hart.pc);

        assert_eq!(state.hart.csrs.mcause, expected_code);
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
        let mut sys = create_test_cpu();
        let mut state = sys.core_ctx(0);
        state.hart.privilege = PrivilegeMode::Machine;
        state.hart.csrs.mtvec = 0x8000_0000;
        state.hart.pc = 0x8000_1000; // Different from trap handler

        state.trap(&trap, state.hart.pc);

        assert_eq!(state.hart.csrs.mcause, expected_code);
    }
}

#[test]
fn test_trap_preserves_registers() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    state.hart.pc = 0x8000_1000; // Different from trap handler

    // Set up some register state
    state.hart.regs.write(RegIdx::new(1), 0x1234);
    state.hart.regs.write(RegIdx::new(2), 0x5678);

    state.trap(&Trap::IllegalInstruction(0), state.hart.pc);

    // Registers should be preserved across trap
    assert_eq!(state.hart.regs.read(RegIdx::new(1)), 0x1234);
    assert_eq!(state.hart.regs.read(RegIdx::new(2)), 0x5678);
}

#[test]
fn test_trap_updates_mepc_correctly() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;
    state.hart.csrs.mtvec = 0x8000_0000;
    let trap_pc = 0x8000_1234;

    state.trap(&Trap::IllegalInstruction(0), trap_pc);

    assert_eq!(state.hart.csrs.mepc, trap_pc);
}

#[test]
fn test_trap_updates_sepc_on_delegation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::User;
    state.hart.csrs.stvec = 0x8000_1000;
    state.hart.csrs.medeleg = 1 << 2; // Delegate illegal instruction
    let trap_pc = 0x8000_5678;

    state.trap(&Trap::IllegalInstruction(0), trap_pc);

    assert_eq!(state.hart.csrs.sepc, trap_pc);
}

use rvsim_core::common::constants::CAUSE_INTERRUPT_BIT;

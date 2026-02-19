//! # Error and Trap Tests
//!
//! This module contains unit tests for trap types, translation results,
//! and error handling mechanisms.

use rvsim_core::common::{PhysAddr, TranslationResult, Trap};

#[test]
fn test_trap_instruction_address_misaligned_display() {
    let trap = Trap::InstructionAddressMisaligned(0x8000_0001);
    assert!(format!("{}", trap).contains("InstructionAddressMisaligned"));
}

#[test]
fn test_trap_instruction_access_fault_display() {
    let trap = Trap::InstructionAccessFault(0xDEAD_BEEF);
    assert!(format!("{}", trap).contains("InstructionAccessFault"));
}

#[test]
fn test_trap_illegal_instruction_display() {
    let trap = Trap::IllegalInstruction(0x12345678);
    assert!(format!("{}", trap).contains("IllegalInstruction"));
}

#[test]
fn test_trap_breakpoint_display() {
    let trap = Trap::Breakpoint(0x8000_0000);
    assert!(format!("{}", trap).contains("Breakpoint"));
}

#[test]
fn test_trap_load_address_misaligned_display() {
    let trap = Trap::LoadAddressMisaligned(0x1000_0003);
    assert!(format!("{}", trap).contains("LoadAddressMisaligned"));
}

#[test]
fn test_trap_load_access_fault_display() {
    let trap = Trap::LoadAccessFault(0xCAFE_BABE);
    assert!(format!("{}", trap).contains("LoadAccessFault"));
}

#[test]
fn test_trap_store_address_misaligned_display() {
    let trap = Trap::StoreAddressMisaligned(0x2000_0001);
    assert!(format!("{}", trap).contains("StoreAddressMisaligned"));
}

#[test]
fn test_trap_store_access_fault_display() {
    let trap = Trap::StoreAccessFault(0xFEED_FACE);
    assert!(format!("{}", trap).contains("StoreAccessFault"));
}

#[test]
fn test_trap_ecall_from_umode_display() {
    let trap = Trap::EnvironmentCallFromUMode;
    assert_eq!(format!("{}", trap), "EnvironmentCallFromUMode");
}

#[test]
fn test_trap_ecall_from_smode_display() {
    let trap = Trap::EnvironmentCallFromSMode;
    assert_eq!(format!("{}", trap), "EnvironmentCallFromSMode");
}

#[test]
fn test_trap_ecall_from_mmode_display() {
    let trap = Trap::EnvironmentCallFromMMode;
    assert_eq!(format!("{}", trap), "EnvironmentCallFromMMode");
}

#[test]
fn test_trap_instruction_page_fault_display() {
    let trap = Trap::InstructionPageFault(0x1000_0000);
    assert!(format!("{}", trap).contains("InstructionPageFault"));
}

#[test]
fn test_trap_load_page_fault_display() {
    let trap = Trap::LoadPageFault(0x2000_0000);
    assert!(format!("{}", trap).contains("LoadPageFault"));
}

#[test]
fn test_trap_store_page_fault_display() {
    let trap = Trap::StorePageFault(0x3000_0000);
    assert!(format!("{}", trap).contains("StorePageFault"));
}

#[test]
fn test_trap_user_software_interrupt_display() {
    let trap = Trap::UserSoftwareInterrupt;
    assert_eq!(format!("{}", trap), "UserSoftwareInterrupt");
}

#[test]
fn test_trap_supervisor_software_interrupt_display() {
    let trap = Trap::SupervisorSoftwareInterrupt;
    assert_eq!(format!("{}", trap), "SupervisorSoftwareInterrupt");
}

#[test]
fn test_trap_machine_software_interrupt_display() {
    let trap = Trap::MachineSoftwareInterrupt;
    assert_eq!(format!("{}", trap), "MachineSoftwareInterrupt");
}

#[test]
fn test_trap_machine_timer_interrupt_display() {
    let trap = Trap::MachineTimerInterrupt;
    assert_eq!(format!("{}", trap), "MachineTimerInterrupt");
}

#[test]
fn test_trap_supervisor_timer_interrupt_display() {
    let trap = Trap::SupervisorTimerInterrupt;
    assert_eq!(format!("{}", trap), "SupervisorTimerInterrupt");
}

#[test]
fn test_trap_machine_external_interrupt_display() {
    let trap = Trap::MachineExternalInterrupt;
    assert_eq!(format!("{}", trap), "MachineExternalInterrupt");
}

#[test]
fn test_trap_supervisor_external_interrupt_display() {
    let trap = Trap::SupervisorExternalInterrupt;
    assert_eq!(format!("{}", trap), "SupervisorExternalInterrupt");
}

#[test]
fn test_trap_user_external_interrupt_display() {
    let trap = Trap::UserExternalInterrupt;
    assert_eq!(format!("{}", trap), "UserExternalInterrupt");
}

#[test]
fn test_trap_requested_trap_display() {
    let trap = Trap::RequestedTrap(42);
    assert_eq!(format!("{}", trap), "RequestedTrap(42)");
}

#[test]
fn test_trap_double_fault_display() {
    let trap = Trap::DoubleFault(0x8000_0100);
    assert_eq!(format!("{}", trap), "DoubleFault(0x80000100)");
}

#[test]
fn test_trap_equality() {
    let trap1 = Trap::IllegalInstruction(0x1234);
    let trap2 = Trap::IllegalInstruction(0x1234);
    let trap3 = Trap::IllegalInstruction(0x5678);

    assert_eq!(trap1, trap2);
    assert_ne!(trap1, trap3);
}

#[test]
fn test_trap_clone() {
    let trap = Trap::LoadAccessFault(0xDEAD_BEEF);
    let cloned = trap.clone();
    assert_eq!(trap, cloned);
}

#[test]
fn test_trap_debug_format() {
    let trap = Trap::InstructionAccessFault(0x1000);
    let debug_str = format!("{:?}", trap);
    assert!(debug_str.contains("InstructionAccessFault"));
}

#[test]
fn test_translation_result_success() {
    let paddr = PhysAddr::new(0x8000_0000);
    let result = TranslationResult::success(paddr, 10);

    assert_eq!(result.paddr, paddr);
    assert_eq!(result.cycles, 10);
    assert_eq!(result.trap, None);
}

#[test]
fn test_translation_result_fault() {
    let trap = Trap::InstructionPageFault(0x1000_0000);
    let result = TranslationResult::fault(trap.clone(), 5);

    assert_eq!(result.paddr, PhysAddr::new(0));
    assert_eq!(result.cycles, 5);
    assert_eq!(result.trap, Some(trap));
}

#[test]
fn test_translation_result_with_zero_cycles() {
    let paddr = PhysAddr::new(0x1000);
    let result = TranslationResult::success(paddr, 0);

    assert_eq!(result.cycles, 0);
    assert_eq!(result.trap, None);
}

#[test]
fn test_translation_result_with_large_cycles() {
    let paddr = PhysAddr::new(0x8000_0000);
    let result = TranslationResult::success(paddr, 1000000);

    assert_eq!(result.cycles, 1000000);
}

#[test]
fn test_translation_result_fault_preserves_trap() {
    let trap = Trap::StorePageFault(0xDEAD_BEEF);
    let result = TranslationResult::fault(trap.clone(), 15);

    match result.trap {
        Some(Trap::StorePageFault(addr)) => {
            assert_eq!(addr, 0xDEAD_BEEF);
        }
        _ => panic!("Expected StorePageFault trap"),
    }
}

#[test]
fn test_trap_is_error() {
    use std::error::Error;
    let trap = Trap::IllegalInstruction(0);
    let _: &dyn Error = &trap;
    // If this compiles, the trap implements Error trait
}

#[test]
fn test_all_trap_variants() {
    let traps = vec![
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
        Trap::InstructionPageFault(0),
        Trap::LoadPageFault(0),
        Trap::StorePageFault(0),
        Trap::UserSoftwareInterrupt,
        Trap::SupervisorSoftwareInterrupt,
        Trap::MachineSoftwareInterrupt,
        Trap::MachineTimerInterrupt,
        Trap::SupervisorTimerInterrupt,
        Trap::MachineExternalInterrupt,
        Trap::SupervisorExternalInterrupt,
        Trap::UserExternalInterrupt,
        Trap::RequestedTrap(0),
        Trap::DoubleFault(0),
    ];

    // All traps should be displayable
    for trap in traps {
        let _ = format!("{}", trap);
    }
}

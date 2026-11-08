//! Traps: how they print, the cause they report, and translation results
//! that carry them.

use crate::arch::translation::TranslationResult;
use crate::common::PhysAddr;
use crate::isa::privileged::{Trap, TrapCause};

#[test]
fn every_trap_prints_its_name_and_value() {
    let cases = [
        (
            Trap::InstructionAddressMisaligned(0x8000_0001),
            "InstructionAddressMisaligned(0x80000001)",
        ),
        (Trap::InstructionAccessFault(0xDEAD_BEEF), "InstructionAccessFault(0xdeadbeef)"),
        (Trap::IllegalInstruction(0x1234_5678), "IllegalInstruction(0x12345678)"),
        (Trap::Breakpoint(0x8000_0000), "Breakpoint(0x80000000)"),
        (Trap::LoadAddressMisaligned(0x1000_0003), "LoadAddressMisaligned(0x10000003)"),
        (Trap::LoadAccessFault(0xCAFE_BABE), "LoadAccessFault(0xcafebabe)"),
        (Trap::StoreAddressMisaligned(0x2000_0001), "StoreAddressMisaligned(0x20000001)"),
        (Trap::StoreAccessFault(0xFEED_FACE), "StoreAccessFault(0xfeedface)"),
        (Trap::EnvironmentCallFromUMode, "EnvironmentCallFromUMode"),
        (Trap::EnvironmentCallFromSMode, "EnvironmentCallFromSMode"),
        (Trap::EnvironmentCallFromMMode, "EnvironmentCallFromMMode"),
        (Trap::InstructionPageFault(0x1000), "InstructionPageFault(0x1000)"),
        (Trap::LoadPageFault(0x2000), "LoadPageFault(0x2000)"),
        (Trap::StorePageFault(0x3000), "StorePageFault(0x3000)"),
        (Trap::UserSoftwareInterrupt, "UserSoftwareInterrupt"),
        (Trap::SupervisorSoftwareInterrupt, "SupervisorSoftwareInterrupt"),
        (Trap::MachineSoftwareInterrupt, "MachineSoftwareInterrupt"),
        (Trap::MachineTimerInterrupt, "MachineTimerInterrupt"),
        (Trap::SupervisorTimerInterrupt, "SupervisorTimerInterrupt"),
        (Trap::MachineExternalInterrupt, "MachineExternalInterrupt"),
        (Trap::SupervisorExternalInterrupt, "SupervisorExternalInterrupt"),
        (Trap::UserExternalInterrupt, "UserExternalInterrupt"),
        (Trap::RequestedTrap(42), "RequestedTrap(42)"),
        (Trap::DoubleFault(0x8000_0100), "DoubleFault(0x80000100)"),
    ];

    for (trap, printed) in cases {
        assert_eq!(trap.to_string(), printed);
    }
}

#[test]
fn a_trap_is_an_error() {
    fn takes_an_error(_: &dyn std::error::Error) {}

    takes_an_error(&Trap::IllegalInstruction(0));
}

#[test]
fn a_successful_translation_carries_its_address_and_cycles() {
    let paddr = PhysAddr::new(0x8000_0000);

    let result = TranslationResult::success(paddr, 10);

    assert_eq!((result.paddr, result.cycles, result.trap), (paddr, 10, None));
}

#[test]
fn a_failed_translation_carries_its_trap_and_cycles() {
    let trap = Trap::StorePageFault(0xDEAD_BEEF);

    let result = TranslationResult::fault(trap.clone(), 5);

    assert_eq!((result.paddr, result.cycles, result.trap), (PhysAddr::new(0), 5, Some(trap)));
}

#[test]
fn mcause_code_sets_the_interrupt_bit_only_for_interrupts() {
    assert_eq!(Trap::EnvironmentCallFromSMode.mcause_code(), 9);
    assert_eq!(Trap::MachineTimerInterrupt.mcause_code(), (1 << 63) | 7);
    assert_eq!(Trap::LoadPageFault(0).mcause_code(), 13);
}

#[test]
fn cause_separates_interrupts_from_exceptions() {
    assert_eq!(Trap::MachineTimerInterrupt.cause(), TrapCause::Interrupt(7));
    assert_eq!(Trap::SupervisorExternalInterrupt.cause(), TrapCause::Interrupt(9));
    assert_eq!(Trap::IllegalInstruction(0).cause(), TrapCause::Exception(2));
    assert_eq!(Trap::RequestedTrap(24).cause(), TrapCause::Exception(24));
}

#[test]
fn only_timer_interrupts_and_ecalls_are_routine() {
    assert!(Trap::SupervisorTimerInterrupt.is_routine());
    assert!(Trap::EnvironmentCallFromUMode.is_routine());
    assert!(!Trap::MachineExternalInterrupt.is_routine());
    assert!(!Trap::IllegalInstruction(0).is_routine());
}

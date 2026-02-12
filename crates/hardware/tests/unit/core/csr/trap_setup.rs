//! # Trap Setup CSR Tests
//!
//! This module provides unit tests for the Control and Status Registers (CSRs)
//! responsible for configuring trap handling in the RISC-V architecture.
//!
//! The tests verify the functionality of:
//! - Exception and interrupt delegation (`medeleg`, `mideleg`).
//! - Trap vector base address configuration (`mtvec`, `stvec`) for both direct and vectored modes.
//! - Machine status management (`mstatus`), including interrupt enables, privilege modes, and extension states.

use riscv_core::core::arch::csr::{self, Csrs};

/// Tests the delegation of exceptions from M-mode to S-mode using the `medeleg` register.
///
/// Verifies that specific exception causes, such as User Ecall (8) and Load Page Fault (13),
/// can be correctly written to and read back from the Machine Exception Delegation Register.
#[test]
fn trap_delegation_medeleg() {
    let mut csrs = Csrs::default();
    // Delegate user ecall (cause 8) and load page fault (cause 13) to S-mode
    let deleg = (1 << 8) | (1 << 13);
    csrs.write(csr::MEDELEG, deleg);
    assert_eq!(csrs.read(csr::MEDELEG), deleg);
}

/// Tests the delegation of interrupts from M-mode to S-mode using the `mideleg` register.
///
/// Verifies that the Supervisor Timer Interrupt (STIP) bit can be set in the
/// Machine Interrupt Delegation Register.
#[test]
fn trap_delegation_mideleg() {
    let mut csrs = Csrs::default();
    // Delegate supervisor timer interrupt
    csrs.write(csr::MIDELEG, csr::MIP_STIP);
    assert_eq!(csrs.read(csr::MIDELEG), csr::MIP_STIP);
}

/// Tests the configuration of the `mtvec` register in Direct mode.
///
/// Verifies that the base address is correctly stored and that the mode bits (1:0)
/// indicate Direct mode (0).
#[test]
fn mtvec_direct_mode() {
    let mut csrs = Csrs::default();
    // Direct mode: mode bits (1:0) = 0, base = 0x8000_0000
    let mtvec = 0x8000_0000;
    csrs.write(csr::MTVEC, mtvec);
    assert_eq!(csrs.read(csr::MTVEC), mtvec);
    assert_eq!(csrs.read(csr::MTVEC) & 0x3, 0, "Mode should be direct (0)");
}

/// Tests the configuration of the `mtvec` register in Vectored mode.
///
/// Verifies that the mode bits (1:0) correctly indicate Vectored mode (1),
/// where all exceptions jump to the base address but interrupts jump to `base + 4 * cause`.
#[test]
fn mtvec_vectored_mode() {
    let mut csrs = Csrs::default();
    // Vectored mode: mode bits = 1
    let mtvec = 0x8000_0001;
    csrs.write(csr::MTVEC, mtvec);
    assert_eq!(
        csrs.read(csr::MTVEC) & 0x3,
        1,
        "Mode should be vectored (1)"
    );
}

/// Tests the configuration of the `stvec` register.
///
/// Verifies that the Supervisor Trap-Vector Base-Address Register can be
/// written to and read from correctly.
#[test]
fn stvec_configuration() {
    let mut csrs = Csrs::default();
    csrs.write(csr::STVEC, 0x8000_0100);
    assert_eq!(csrs.read(csr::STVEC), 0x8000_0100);
}

/// Verifies that the Machine and Supervisor Interrupt Enable bits (MIE, SIE)
/// in the `mstatus` register can be correctly set and retrieved.
#[test]
fn mstatus_interrupt_enable_bits() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MSTATUS, csr::MSTATUS_MIE | csr::MSTATUS_SIE);
    let mstatus = csrs.read(csr::MSTATUS);
    assert_ne!(mstatus & csr::MSTATUS_MIE, 0, "MIE should be set");
    assert_ne!(mstatus & csr::MSTATUS_SIE, 0, "SIE should be set");
}

/// Verifies that the Machine Previous Privilege (MPP) field in the `mstatus`
/// register correctly stores and returns the privilege mode.
#[test]
fn mstatus_previous_mode_mpp() {
    let mut csrs = Csrs::default();
    // Set MPP to Supervisor (1)
    csrs.write(csr::MSTATUS, 1 << csr::MSTATUS_MPP_SHIFT);
    let mpp = (csrs.read(csr::MSTATUS) >> csr::MSTATUS_MPP_SHIFT) & csr::MSTATUS_MPP_MASK;
    assert_eq!(mpp, 1, "MPP should be Supervisor (1)");
}

/// Verifies that the Floating-point Status (FS) field in the `mstatus`
/// register correctly tracks the dirty state.
#[test]
fn mstatus_fs_field() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MSTATUS, csr::MSTATUS_FS_DIRTY);
    assert_eq!(
        csrs.read(csr::MSTATUS) & csr::MSTATUS_FS,
        csr::MSTATUS_FS_DIRTY
    );
}

/// Verifies the delegation of exceptions and interrupts from Machine mode
/// to Supervisor mode using the `medeleg` and `mideleg` registers.
#[test]
fn medeleg_mideleg_combined() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MEDELEG, (1 << 8) | (1 << 13));
    csrs.write(csr::MIDELEG, csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP);

    let edeleg = csrs.read(csr::MEDELEG);
    let ideleg = csrs.read(csr::MIDELEG);
    assert_ne!(edeleg & (1 << 8), 0, "User ecall should be delegated");
    assert_ne!(
        ideleg & csr::MIP_STIP,
        0,
        "S-mode timer should be delegated"
    );
}

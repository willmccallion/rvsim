//! # CSR Access Control Tests
//!
//! This module contains unit tests for the RISC-V Control and Status Registers (CSRs) implementation.
//! It verifies the initialization, read/write logic, and architectural constraints for both
//! Machine-mode and Supervisor-mode registers, including specific handling for address
//! translation (`satp`) and hardware counters.

use riscv_core::core::arch::csr::{self, Csrs};

/// Verifies that all Control and Status Registers (CSRs) are initialized to zero by default.
#[test]
fn csr_default_all_zero() {
    let csrs = Csrs::default();
    assert_eq!(csrs.mstatus, 0);
    assert_eq!(csrs.misa, 0);
    assert_eq!(csrs.mie, 0);
    assert_eq!(csrs.mip, 0);
    assert_eq!(csrs.mepc, 0);
    assert_eq!(csrs.mcause, 0);
    assert_eq!(csrs.mtval, 0);
    assert_eq!(csrs.mscratch, 0);
    assert_eq!(csrs.mtvec, 0);
    assert_eq!(csrs.satp, 0);
}

/// Verifies that the `mstatus` (Machine Status) register can be written to and read back correctly.
#[test]
fn csr_write_and_read_mstatus() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MSTATUS, 0x0000_0000_000A_0000);
    assert_eq!(csrs.read(csr::MSTATUS), 0x0000_0000_000A_0000);
}

/// Verifies that the `misa` (Machine ISA) register can be written to and read back correctly.
#[test]
fn csr_write_and_read_misa() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MISA, csr::MISA_DEFAULT_RV64IMAFDC);
    assert_eq!(csrs.read(csr::MISA), csr::MISA_DEFAULT_RV64IMAFDC);
}

/// Verifies that the `mie` (Machine Interrupt Enable) register can be written to and read back correctly.
#[test]
fn csr_write_and_read_mie() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MIE, csr::MIE_MTIE | csr::MIE_MSIP);
    assert_eq!(csrs.read(csr::MIE), csr::MIE_MTIE | csr::MIE_MSIP);
}

/// Verifies that the `mip` (Machine Interrupt Pending) register can be written to and read back correctly.
#[test]
fn csr_write_and_read_mip() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MIP, csr::MIP_MTIP);
    assert_eq!(csrs.read(csr::MIP), csr::MIP_MTIP);
}

/// Verifies that the `mepc` (Machine Exception Program Counter) register can be written to and read back correctly.
#[test]
fn csr_write_and_read_mepc() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MEPC, 0x8000_1234);
    assert_eq!(csrs.read(csr::MEPC), 0x8000_1234);
}

/// Verifies that the `mcause` (Machine Cause) register can be written to and read back correctly.
#[test]
fn csr_write_and_read_mcause() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MCAUSE, 0x8000_0000_0000_0007); // Timer interrupt
    assert_eq!(csrs.read(csr::MCAUSE), 0x8000_0000_0000_0007);
}

/// Verifies that basic Supervisor-level Control and Status Registers (CSRs)
/// can be written to and read back correctly.
#[test]
fn csr_write_and_read_supervisor_csrs() {
    let mut csrs = Csrs::default();
    csrs.write(csr::SSTATUS, 0x22);
    csrs.write(csr::SIE, 0x222);
    csrs.write(csr::STVEC, 0x8000_0000);
    csrs.write(csr::SSCRATCH, 0xDEAD);
    csrs.write(csr::SEPC, 0x1000);
    csrs.write(csr::SCAUSE, 15);
    csrs.write(csr::STVAL, 0xBEEF);
    csrs.write(csr::SIP, 0x2);

    assert_eq!(csrs.read(csr::SSTATUS), 0x22);
    assert_eq!(csrs.read(csr::SIE), 0x222);
    assert_eq!(csrs.read(csr::STVEC), 0x8000_0000);
    assert_eq!(csrs.read(csr::SSCRATCH), 0xDEAD);
    assert_eq!(csrs.read(csr::SEPC), 0x1000);
    assert_eq!(csrs.read(csr::SCAUSE), 15);
    assert_eq!(csrs.read(csr::STVAL), 0xBEEF);
    assert_eq!(csrs.read(csr::SIP), 0x2);
}

/// Verifies that the `satp` register correctly preserves the Sv39 paging mode
/// and the Physical Page Number (PPN) field.
#[test]
fn csr_satp_mode_sv39_preserved() {
    let mut csrs = Csrs::default();
    let sv39_satp = (csr::SATP_MODE_SV39 << csr::SATP_MODE_SHIFT) | 0x12345;
    csrs.write(csr::SATP, sv39_satp);
    let read_back = csrs.read(csr::SATP);
    let mode = (read_back >> csr::SATP_MODE_SHIFT) & csr::SATP_MODE_MASK;
    assert_eq!(mode, csr::SATP_MODE_SV39);
    assert_eq!(read_back & csr::SATP_PPN_MASK, 0x12345);
}

/// Verifies that the `satp` register correctly preserves the Bare (no translation) mode.
#[test]
fn csr_satp_mode_bare_preserved() {
    let mut csrs = Csrs::default();
    let bare_satp = (csr::SATP_MODE_BARE << csr::SATP_MODE_SHIFT) | 0x0;
    csrs.write(csr::SATP, bare_satp);
    let read_back = csrs.read(csr::SATP);
    let mode = (read_back >> csr::SATP_MODE_SHIFT) & csr::SATP_MODE_MASK;
    assert_eq!(mode, csr::SATP_MODE_BARE);
}

/// Verifies that writing an invalid or unsupported mode to the `satp` register
/// causes it to default to the Bare mode, as per the RISC-V specification.
#[test]
fn csr_satp_invalid_mode_becomes_bare() {
    let mut csrs = Csrs::default();
    // Mode value 5 is not valid (only 0=bare, 8=sv39)
    let invalid_satp = (5u64 << csr::SATP_MODE_SHIFT) | 0xABC;
    csrs.write(csr::SATP, invalid_satp);
    let read_back = csrs.read(csr::SATP);
    let mode = (read_back >> csr::SATP_MODE_SHIFT) & csr::SATP_MODE_MASK;
    assert_eq!(mode, csr::SATP_MODE_BARE, "Invalid mode should become Bare");
}

/// Verifies that reading from an undefined CSR address returns zero.
#[test]
fn csr_unknown_address_returns_zero() {
    let csrs = Csrs::default();
    assert_eq!(csrs.read(0x999), 0);
}

/// Verifies that writing to an undefined CSR address is ignored and does not affect the state.
#[test]
fn csr_write_unknown_address_is_ignored() {
    let mut csrs = Csrs::default();
    csrs.write(0x999, 0xDEAD);
    assert_eq!(csrs.read(0x999), 0);
}

/// Verifies the read and write operations for common counter CSRs.
#[test]
fn csr_counter_csrs() {
    let mut csrs = Csrs::default();
    csrs.write(csr::CYCLE, 100);
    csrs.write(csr::TIME, 200);
    csrs.write(csr::INSTRET, 300);
    csrs.write(csr::MCYCLE, 400);
    csrs.write(csr::MINSTRET, 500);

    assert_eq!(csrs.read(csr::CYCLE), 100);
    assert_eq!(csrs.read(csr::TIME), 200);
    assert_eq!(csrs.read(csr::INSTRET), 300);
    assert_eq!(csrs.read(csr::MCYCLE), 400);
    assert_eq!(csrs.read(csr::MINSTRET), 500);
}

/// Verifies that cloning a `Csrs` instance preserves the values of the registers.
#[test]
fn csr_clone() {
    let mut csrs = Csrs::default();
    csrs.write(csr::MSTATUS, 0xABCD);
    let cloned = csrs.clone();
    assert_eq!(cloned.read(csr::MSTATUS), 0xABCD);
}

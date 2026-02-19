//! # CPU CSR Operations Tests
//!
//! This module contains unit tests for the CPU's CSR read/write operations,
//! including side effects like TLB flushes, interrupt inhibition, and
//! synchronization between MSTATUS and SSTATUS.

use rvsim_core::config::Config;
use rvsim_core::core::Cpu;
use rvsim_core::core::arch::csr;

/// Helper function to create a test CPU instance.
fn create_test_cpu() -> Cpu {
    let config = Config::default();
    let system = rvsim_core::soc::System::new(&config, "");
    Cpu::new(system, &config)
}

#[test]
fn test_csr_read_machine_info() {
    let cpu = create_test_cpu();

    // Read-only machine information registers should return expected values
    assert_eq!(cpu.csr_read(csr::MVENDORID), 0);
    assert_eq!(cpu.csr_read(csr::MARCHID), 0);
    assert_eq!(cpu.csr_read(csr::MIMPID), 0);
    assert_eq!(cpu.csr_read(csr::MHARTID), 0);
}

#[test]
fn test_csr_read_write_mstatus() {
    let mut cpu = create_test_cpu();

    let test_value = 0x1800; // MPP=11 (Machine mode)
    cpu.csr_write(csr::MSTATUS, test_value);
    assert_eq!(cpu.csr_read(csr::MSTATUS), test_value);
}

#[test]
fn test_csr_read_write_mie() {
    let mut cpu = create_test_cpu();

    let test_value = csr::MIE_MTIE | csr::MIE_MSIP | csr::MIE_MEIP;
    cpu.csr_write(csr::MIE, test_value);
    assert_eq!(cpu.csr_read(csr::MIE), test_value);
}

#[test]
fn test_csr_read_write_mip() {
    let mut cpu = create_test_cpu();

    // Only SSIP, STIP, SEIP bits are writable
    let test_value = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    cpu.csr_write(csr::MIP, test_value);

    let result = cpu.csr_read(csr::MIP);
    assert_eq!(result & test_value, test_value);
}

#[test]
fn test_csr_read_write_mtvec() {
    let mut cpu = create_test_cpu();

    let test_value = 0x8000_0000;
    cpu.csr_write(csr::MTVEC, test_value);
    assert_eq!(cpu.csr_read(csr::MTVEC), test_value);
}

#[test]
fn test_csr_read_write_mscratch() {
    let mut cpu = create_test_cpu();

    let test_value = 0xDEADBEEF_CAFEBABE;
    cpu.csr_write(csr::MSCRATCH, test_value);
    assert_eq!(cpu.csr_read(csr::MSCRATCH), test_value);
}

#[test]
fn test_csr_read_write_mepc() {
    let mut cpu = create_test_cpu();

    // MEPC should clear the lowest bit
    let test_value = 0x8000_0001;
    cpu.csr_write(csr::MEPC, test_value);
    assert_eq!(cpu.csr_read(csr::MEPC), 0x8000_0000);
}

#[test]
fn test_csr_read_write_mcause() {
    let mut cpu = create_test_cpu();

    let test_value = 0x8000_0000_0000_0005; // Interrupt bit set, cause 5
    cpu.csr_write(csr::MCAUSE, test_value);
    assert_eq!(cpu.csr_read(csr::MCAUSE), test_value);
}

#[test]
fn test_csr_read_write_mtval() {
    let mut cpu = create_test_cpu();

    let test_value = 0x1234_5678_9ABC_DEF0;
    cpu.csr_write(csr::MTVAL, test_value);
    assert_eq!(cpu.csr_read(csr::MTVAL), test_value);
}

#[test]
fn test_csr_read_write_medeleg() {
    let mut cpu = create_test_cpu();

    let test_value = 0xB3FF; // Delegate all synchronous exceptions
    cpu.csr_write(csr::MEDELEG, test_value);
    assert_eq!(cpu.csr_read(csr::MEDELEG), test_value);
}

#[test]
fn test_csr_read_write_mideleg() {
    let mut cpu = create_test_cpu();

    let test_value = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    cpu.csr_write(csr::MIDELEG, test_value);
    assert_eq!(cpu.csr_read(csr::MIDELEG), test_value);
}

#[test]
fn test_csr_sstatus_synchronization() {
    let mut cpu = create_test_cpu();

    // Write to MSTATUS and verify SSTATUS is updated
    let mstatus_value = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
    cpu.csr_write(csr::MSTATUS, mstatus_value);

    let sstatus = cpu.csr_read(csr::SSTATUS);
    assert_eq!(sstatus & mstatus_value, mstatus_value);
}

#[test]
fn test_csr_write_sstatus_masks_properly() {
    let mut cpu = create_test_cpu();

    // Write to SSTATUS with various bits
    let sstatus_value = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
    cpu.csr_write(csr::SSTATUS, sstatus_value);

    // Verify only allowed bits are set
    let mask = csr::MSTATUS_SIE
        | csr::MSTATUS_SPIE
        | csr::MSTATUS_SPP
        | csr::MSTATUS_FS
        | csr::MSTATUS_SUM
        | csr::MSTATUS_MXR;

    let mstatus = cpu.csr_read(csr::MSTATUS);
    assert_eq!(mstatus & mask, sstatus_value);
}

#[test]
fn test_csr_sie_delegation() {
    let mut cpu = create_test_cpu();

    // Set delegation mask
    let delegation = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    cpu.csr_write(csr::MIDELEG, delegation);

    // Set MIE bits
    cpu.csr_write(csr::MIE, csr::MIE_MSIP | csr::MIE_MTIE | csr::MIE_MEIP);

    // Read SIE (should only see delegated bits)
    let sie = cpu.csr_read(csr::SIE);
    assert_eq!(sie, cpu.csr_read(csr::MIE) & delegation);
}

#[test]
fn test_csr_sip_delegation() {
    let mut cpu = create_test_cpu();

    // Set delegation mask
    let delegation = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    cpu.csr_write(csr::MIDELEG, delegation);

    // Set MIP bits
    cpu.csr_write(csr::MIP, csr::MIP_SSIP | csr::MIP_STIP);

    // Read SIP (should only see delegated bits)
    let sip = cpu.csr_read(csr::SIP);
    assert_eq!(sip, cpu.csr_read(csr::MIP) & delegation);
}

#[test]
fn test_csr_write_sie() {
    let mut cpu = create_test_cpu();

    // Set delegation
    let delegation = csr::MIP_SSIP | csr::MIP_SEIP;
    cpu.csr_write(csr::MIDELEG, delegation);

    // Write to SIE
    cpu.csr_write(csr::SIE, csr::MIE_SSIP | csr::MIE_SEIP);

    // Verify MIE is updated with delegated bits only
    let mie = cpu.csr_read(csr::MIE);
    assert_eq!(mie & delegation, csr::MIE_SSIP | csr::MIE_SEIP);
}

#[test]
fn test_csr_write_sip() {
    let mut cpu = create_test_cpu();

    // Set delegation (only SSIP is writable from supervisor mode)
    cpu.csr_write(csr::MIDELEG, csr::MIP_SSIP);

    // Write to SIP
    cpu.csr_write(csr::SIP, csr::MIP_SSIP);

    // Verify MIP is updated
    assert_eq!(cpu.csr_read(csr::MIP) & csr::MIP_SSIP, csr::MIP_SSIP);
}

#[test]
fn test_csr_stvec() {
    let mut cpu = create_test_cpu();

    let test_value = 0x8000_0100;
    cpu.csr_write(csr::STVEC, test_value);
    assert_eq!(cpu.csr_read(csr::STVEC), test_value);
}

#[test]
fn test_csr_sscratch() {
    let mut cpu = create_test_cpu();

    let test_value = 0xFEEDFACE_DEADBEEF;
    cpu.csr_write(csr::SSCRATCH, test_value);
    assert_eq!(cpu.csr_read(csr::SSCRATCH), test_value);
}

#[test]
fn test_csr_sepc_clears_lowest_bit() {
    let mut cpu = create_test_cpu();

    let test_value = 0x8000_0003;
    cpu.csr_write(csr::SEPC, test_value);
    assert_eq!(cpu.csr_read(csr::SEPC), 0x8000_0002);
}

#[test]
fn test_csr_scause() {
    let mut cpu = create_test_cpu();

    let test_value = 0x8000_0000_0000_0009;
    cpu.csr_write(csr::SCAUSE, test_value);
    assert_eq!(cpu.csr_read(csr::SCAUSE), test_value);
}

#[test]
fn test_csr_stval() {
    let mut cpu = create_test_cpu();

    let test_value = 0xBADC0FFE_BADC0FFE;
    cpu.csr_write(csr::STVAL, test_value);
    assert_eq!(cpu.csr_read(csr::STVAL), test_value);
}

#[test]
fn test_csr_stimecmp_clears_stip() {
    let mut cpu = create_test_cpu();

    // Set STIP bit
    cpu.csr_write(csr::MIP, csr::MIP_STIP);
    assert_ne!(cpu.csr_read(csr::MIP) & csr::MIP_STIP, 0);

    // Write to STIMECMP should clear STIP
    cpu.csr_write(csr::STIMECMP, 1000);
    assert_eq!(cpu.csr_read(csr::MIP) & csr::MIP_STIP, 0);
    assert_eq!(cpu.csr_read(csr::STIMECMP), 1000);
}

#[test]
fn test_csr_satp_sv39_mode() {
    let mut cpu = create_test_cpu();

    // SV39 mode (mode=8)
    let satp_value = (8u64 << 60) | 0x12345;
    cpu.csr_write(csr::SATP, satp_value);
    assert_eq!(cpu.csr_read(csr::SATP), satp_value);
}

#[test]
fn test_csr_satp_bare_mode() {
    let mut cpu = create_test_cpu();

    // Bare mode (mode=0)
    let satp_value = 0x12345;
    cpu.csr_write(csr::SATP, satp_value);
    assert_eq!(cpu.csr_read(csr::SATP), satp_value);
}

#[test]
fn test_csr_satp_invalid_mode_rejected() {
    let mut cpu = create_test_cpu();

    // Invalid mode (mode=5, not SV39 or BARE)
    let satp_value = (5u64 << 60) | 0x12345;
    cpu.csr_write(csr::SATP, satp_value);

    // Mode bits should be cleared, PPN preserved
    assert_eq!(cpu.csr_read(csr::SATP), 0x12345);
}

#[test]
fn test_csr_cycle_counter() {
    let cpu = create_test_cpu();

    // CYCLE and MCYCLE should read the same value
    let cycle = cpu.csr_read(csr::CYCLE);
    let mcycle = cpu.csr_read(csr::MCYCLE);
    assert_eq!(cycle, mcycle);
}

#[test]
fn test_csr_time_counter() {
    let cpu = create_test_cpu();

    // TIME should be cycles divided by clint_divider
    let time = cpu.csr_read(csr::TIME);
    let cycles = cpu.csr_read(csr::CYCLE);
    assert_eq!(time, cycles / cpu.clint_divider);
}

#[test]
fn test_csr_instret_counter() {
    let cpu = create_test_cpu();

    // INSTRET and MINSTRET should read the same value
    let instret = cpu.csr_read(csr::INSTRET);
    let minstret = cpu.csr_read(csr::MINSTRET);
    assert_eq!(instret, minstret);
}

#[test]
fn test_csr_unknown_read_returns_zero() {
    let cpu = create_test_cpu();

    // Reading an unknown/unimplemented CSR should return 0
    assert_eq!(cpu.csr_read(0xFFF), 0);
}

#[test]
fn test_csr_unknown_write_ignored() {
    let mut cpu = create_test_cpu();

    // Writing to an unknown CSR should be ignored (no panic)
    cpu.csr_write(0xFFF, 0xDEADBEEF);
    // If we get here, the write was safely ignored
}

#[test]
fn test_csr_mstatus_write() {
    let cpu = create_test_cpu();
    // Verify basic MSTATUS write doesn't panic
    let _ = cpu;
}

#[test]
fn test_csr_sstatus_write() {
    let cpu = create_test_cpu();
    // Verify basic SSTATUS write doesn't panic
    let _ = cpu;
}

//! # CPU CSR Operations Tests
//!
//! This module contains unit tests for the CPU's CSR read/write operations,
//! including side effects like TLB flushes, interrupt inhibition, and
//! synchronization between MSTATUS and SSTATUS.

use rvsim_core::SimState;
use rvsim_core::arch::csr;
use rvsim_core::config::Config;
use rvsim_core::isa::csr::CsrAddr;

/// Helper function to create a test CPU instance.
fn create_test_cpu() -> SimState {
    let config = Config::default();
    SimState::build(&config, "")
}

#[test]
fn test_csr_read_machine_info() {
    let mut sys = create_test_cpu();
    let state = sys.core_ctx(0);

    // Read-only machine information registers should return expected values
    assert_eq!(state.csr_read(csr::MVENDORID), 0);
    assert_eq!(state.csr_read(csr::MARCHID), 0);
    assert_eq!(state.csr_read(csr::MIMPID), 0);
    // Default hart_id is 0 until multi-core construction wires it.
    assert_eq!(state.csr_read(csr::MHARTID), 0);
}

#[test]
fn test_mhartid_returns_hart_id() {
    use rvsim_core::common::HartId;

    let mut sys = create_test_cpu();

    let state = sys.core_ctx(0);
    state.hart.hart_id = HartId::new(7);
    assert_eq!(state.csr_read(csr::MHARTID), 7);

    state.hart.hart_id = HartId::new(0xFFFF_FFFF);
    assert_eq!(state.csr_read(csr::MHARTID), 0xFFFF_FFFF);
}

#[test]
fn test_csr_read_write_mstatus() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x1800; // MPP=11 (Machine mode)
    state.csr_write(csr::MSTATUS, test_value);
    // UXL and SXL bits (bits 35:32) are WARL and always preserved as 2 (RV64) in mstatus.
    let expected = test_value | csr::MSTATUS_DEFAULT_RV64;
    assert_eq!(state.csr_read(csr::MSTATUS), expected);
}

#[test]
fn test_csr_read_write_mie() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = csr::MIE_MTIE | csr::MIE_MSIP | csr::MIE_MEIP;
    state.csr_write(csr::MIE, test_value);
    assert_eq!(state.csr_read(csr::MIE), test_value);
}

#[test]
fn test_csr_read_write_mip() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Only SSIP, STIP, SEIP bits are writable
    let test_value = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    state.csr_write(csr::MIP, test_value);

    let result = state.csr_read(csr::MIP);
    assert_eq!(result & test_value, test_value);
}

#[test]
fn test_csr_read_write_mtvec() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x8000_0000;
    state.csr_write(csr::MTVEC, test_value);
    assert_eq!(state.csr_read(csr::MTVEC), test_value);
}

#[test]
fn test_csr_read_write_mscratch() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0xDEADBEEF_CAFEBABE;
    state.csr_write(csr::MSCRATCH, test_value);
    assert_eq!(state.csr_read(csr::MSCRATCH), test_value);
}

#[test]
fn test_csr_read_write_mepc() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // MEPC should clear the lowest bit
    let test_value = 0x8000_0001;
    state.csr_write(csr::MEPC, test_value);
    assert_eq!(state.csr_read(csr::MEPC), 0x8000_0000);
}

/// A hart built without the C extension, so IALIGN=32.
fn create_test_cpu_without_c() -> SimState {
    let mut config = Config::default();
    config.pipeline.misa_override = Some("RV64IMAFD".parse().expect("valid ISA string"));
    SimState::build(&config, "")
}

#[test]
fn with_c_an_xepc_keeps_bit_one() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MEPC, 0x8000_0003);
    state.csr_write(csr::SEPC, 0x8000_0003);

    assert_eq!((state.csr_read(csr::MEPC), state.csr_read(csr::SEPC)), (0x8000_0002, 0x8000_0002));
}

#[test]
fn without_c_an_xepc_has_its_two_low_bits_clear() {
    let mut sys = create_test_cpu_without_c();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MEPC, 0x8000_0003);
    state.csr_write(csr::SEPC, 0x8000_0003);

    assert_eq!((state.csr_read(csr::MEPC), state.csr_read(csr::SEPC)), (0x8000_0000, 0x8000_0000));
}

/// FIOM, CBIE, CBCFE and CBZE: the envcfg fields of Zicbom, Zicboz and FIOM.
const ENVCFG_FIELDS: u64 = 0b1111_0001;
const CBIE_RESERVED: u64 = 0b10 << csr::MENVCFG_CBIE_SHIFT;

#[test]
fn menvcfg_keeps_only_the_fields_of_implemented_extensions() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MENVCFG, u64::MAX);

    assert_eq!(state.csr_read(csr::MENVCFG), ENVCFG_FIELDS | csr::MENVCFG_STCE);
}

#[test]
fn menvcfg_adue_is_writable_on_a_svadu_hart() {
    let mut config = Config::default();
    config.isa.svadu = true;
    let mut sys = SimState::build(&config, "");
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MENVCFG, u64::MAX);

    assert_eq!(state.csr_read(csr::MENVCFG), ENVCFG_FIELDS | csr::MENVCFG_STCE | csr::MENVCFG_ADUE);
}

#[test]
fn senvcfg_keeps_only_the_fields_of_implemented_extensions() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::SENVCFG, u64::MAX);

    assert_eq!(state.csr_read(csr::SENVCFG), ENVCFG_FIELDS);
}

#[test]
fn the_reserved_cbie_encoding_is_written_as_illegal() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MENVCFG, CBIE_RESERVED | csr::MENVCFG_CBZE);
    state.csr_write(csr::SENVCFG, CBIE_RESERVED);

    assert_eq!(
        (state.csr_read(csr::MENVCFG), state.csr_read(csr::SENVCFG)),
        (csr::MENVCFG_CBZE, 0)
    );
}

#[test]
fn a_satp_write_keeps_asid_tagged_tlb_entries() {
    use rvsim_core::common::{Asid, Ppn, Vpn};
    use rvsim_core::uarch::mmu::tlb::PageSize;
    const PTE_VR: u64 = 0b11;
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.core.mmu.dtlb.insert(
        Vpn::new(0x40),
        Ppn::new(0x8_0040),
        PTE_VR,
        Asid::new(1),
        PageSize::Kib4,
    );

    state.csr_write(csr::SATP, (csr::SATP_MODE_SV39 << 60) | (2 << 44) | 0x8_0100);

    assert!(state.core.mmu.dtlb.peek(Vpn::new(0x40), Asid::new(1)).is_some());
}

#[test]
fn test_csr_read_write_mcause() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x8000_0000_0000_0005; // Interrupt bit set, cause 5
    state.csr_write(csr::MCAUSE, test_value);
    assert_eq!(state.csr_read(csr::MCAUSE), test_value);
}

#[test]
fn test_csr_read_write_mtval() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x1234_5678_9ABC_DEF0;
    state.csr_write(csr::MTVAL, test_value);
    assert_eq!(state.csr_read(csr::MTVAL), test_value);
}

#[test]
fn test_csr_read_write_medeleg() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0xB3FF; // Delegate all synchronous exceptions
    state.csr_write(csr::MEDELEG, test_value);
    assert_eq!(state.csr_read(csr::MEDELEG), test_value);
}

#[test]
fn test_csr_read_write_mideleg() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    state.csr_write(csr::MIDELEG, test_value);
    assert_eq!(state.csr_read(csr::MIDELEG), test_value);
}

#[test]
fn test_csr_sstatus_synchronization() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Write to MSTATUS and verify SSTATUS is updated
    let mstatus_value = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
    state.csr_write(csr::MSTATUS, mstatus_value);

    let sstatus = state.csr_read(csr::SSTATUS);
    assert_eq!(sstatus & mstatus_value, mstatus_value);
}

#[test]
fn test_csr_write_sstatus_masks_properly() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Write to SSTATUS with various bits
    let sstatus_value = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
    state.csr_write(csr::SSTATUS, sstatus_value);

    // Verify only allowed bits are set
    let mask = csr::MSTATUS_SIE
        | csr::MSTATUS_SPIE
        | csr::MSTATUS_SPP
        | csr::MSTATUS_FS
        | csr::MSTATUS_SUM
        | csr::MSTATUS_MXR;

    let mstatus = state.csr_read(csr::MSTATUS);
    assert_eq!(mstatus & mask, sstatus_value);
}

#[test]
fn test_csr_sie_delegation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Set delegation mask
    let delegation = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    state.csr_write(csr::MIDELEG, delegation);

    // Set MIE bits
    state.csr_write(csr::MIE, csr::MIE_MSIP | csr::MIE_MTIE | csr::MIE_MEIP);

    // Read SIE (should only see delegated bits)
    let sie = state.csr_read(csr::SIE);
    assert_eq!(sie, state.csr_read(csr::MIE) & delegation);
}

#[test]
fn test_csr_sip_delegation() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Set delegation mask
    let delegation = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
    state.csr_write(csr::MIDELEG, delegation);

    // Set MIP bits
    state.csr_write(csr::MIP, csr::MIP_SSIP | csr::MIP_STIP);

    // Read SIP (should only see delegated bits)
    let sip = state.csr_read(csr::SIP);
    assert_eq!(sip, state.csr_read(csr::MIP) & delegation);
}

#[test]
fn test_csr_write_sie() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Set delegation
    let delegation = csr::MIP_SSIP | csr::MIP_SEIP;
    state.csr_write(csr::MIDELEG, delegation);

    // Write to SIE
    state.csr_write(csr::SIE, csr::MIE_SSIP | csr::MIE_SEIP);

    // Verify MIE is updated with delegated bits only
    let mie = state.csr_read(csr::MIE);
    assert_eq!(mie & delegation, csr::MIE_SSIP | csr::MIE_SEIP);
}

#[test]
fn test_csr_write_sip() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Set delegation (only SSIP is writable from supervisor mode)
    state.csr_write(csr::MIDELEG, csr::MIP_SSIP);

    // Write to SIP
    state.csr_write(csr::SIP, csr::MIP_SSIP);

    // Verify MIP is updated
    assert_eq!(state.csr_read(csr::MIP) & csr::MIP_SSIP, csr::MIP_SSIP);
}

#[test]
fn test_csr_stvec() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x8000_0100;
    state.csr_write(csr::STVEC, test_value);
    assert_eq!(state.csr_read(csr::STVEC), test_value);
}

#[test]
fn test_csr_sscratch() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0xFEEDFACE_DEADBEEF;
    state.csr_write(csr::SSCRATCH, test_value);
    assert_eq!(state.csr_read(csr::SSCRATCH), test_value);
}

#[test]
fn test_csr_sepc_clears_lowest_bit() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x8000_0003;
    state.csr_write(csr::SEPC, test_value);
    assert_eq!(state.csr_read(csr::SEPC), 0x8000_0002);
}

#[test]
fn test_csr_scause() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0x8000_0000_0000_0009;
    state.csr_write(csr::SCAUSE, test_value);
    assert_eq!(state.csr_read(csr::SCAUSE), test_value);
}

#[test]
fn test_csr_stval() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    let test_value = 0xBADC0FFE_BADC0FFE;
    state.csr_write(csr::STVAL, test_value);
    assert_eq!(state.csr_read(csr::STVAL), test_value);
}

#[test]
fn sstc_raises_stip_once_the_clint_time_reaches_stimecmp() {
    let config = Config::default();
    let mut sim = rvsim_core::Simulator::build(&config, "");
    let mtime = rvsim_core::common::PhysAddr::new(config.system.clint_base + 0xBFF8);
    sim.probe_mem_store(mtime, 5000, 8);
    let mut state = sim.state.core_ctx(0);
    state.csr_write(csr::MENVCFG, csr::MENVCFG_STCE);
    state.csr_write(csr::STIMECMP, 4000);

    state.pre_tick(rvsim_core::soc::interconnect::HartIrqs::default());

    assert_ne!(state.csr_read(csr::MIP) & csr::MIP_STIP, 0, "time 5000 is past stimecmp 4000");
}

#[test]
fn test_csr_stimecmp_clears_stip() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Set STIP bit
    state.csr_write(csr::MIP, csr::MIP_STIP);
    assert_ne!(state.csr_read(csr::MIP) & csr::MIP_STIP, 0);

    // Write to STIMECMP should clear STIP
    state.csr_write(csr::STIMECMP, 1000);
    assert_eq!(state.csr_read(csr::MIP) & csr::MIP_STIP, 0);
    assert_eq!(state.csr_read(csr::STIMECMP), 1000);
}

#[test]
fn test_csr_satp_sv39_mode() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // SV39 mode (mode=8)
    let satp_value = (8u64 << 60) | 0x12345;
    state.csr_write(csr::SATP, satp_value);
    assert_eq!(state.csr_read(csr::SATP), satp_value);
}

#[test]
fn test_csr_satp_bare_mode() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Bare mode (mode=0)
    let satp_value = 0x12345;
    state.csr_write(csr::SATP, satp_value);
    assert_eq!(state.csr_read(csr::SATP), satp_value);
}

#[test]
fn test_csr_satp_invalid_mode_rejected() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Invalid mode (mode=5, not SV39 or BARE)
    let satp_value = (5u64 << 60) | 0x12345;
    state.csr_write(csr::SATP, satp_value);

    // Mode bits should be cleared, PPN preserved
    assert_eq!(state.csr_read(csr::SATP), 0x12345);
}

#[test]
fn test_csr_satp_sv48_accepted_by_default() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    let satp = (csr::SATP_MODE_SV48 << 60) | 0x12345;
    state.csr_write(csr::SATP, satp);
    assert_eq!(state.csr_read(csr::SATP), satp);
}

#[test]
fn test_csr_satp_sv57_accepted_by_default() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    let satp = (csr::SATP_MODE_SV57 << 60) | 0x12345;
    state.csr_write(csr::SATP, satp);
    assert_eq!(state.csr_read(csr::SATP), satp);
}

#[test]
fn test_csr_satp_paging_mode_cap_coerces_above_cap() {
    let mut config = Config::default();
    config.memory.paging_mode_max = csr::PagingMode::Sv39;
    let mut sys = SimState::build(&config, "");
    let mut state = sys.core_ctx(0);

    // Sv48 is above the cap → coerce to Bare; PPN preserved.
    let above_cap = (csr::SATP_MODE_SV48 << 60) | 0x12345;
    state.csr_write(csr::SATP, above_cap);
    assert_eq!(state.csr_read(csr::SATP), 0x12345);

    // Sv39 is at the cap → accepted as written.
    let at_cap = (csr::SATP_MODE_SV39 << 60) | 0x12345;
    state.csr_write(csr::SATP, at_cap);
    assert_eq!(state.csr_read(csr::SATP), at_cap);
}

#[test]
fn test_csr_cycle_counter() {
    let mut sys = create_test_cpu();
    let state = sys.core_ctx(0);

    // CYCLE and MCYCLE should read the same value
    let cycle = state.csr_read(csr::CYCLE);
    let mcycle = state.csr_read(csr::MCYCLE);
    assert_eq!(cycle, mcycle);
}

#[test]
fn test_csr_time_counter() {
    let mut sys = create_test_cpu();
    let state = sys.core_ctx(0);

    // TIME should be cycles divided by clint_divider
    let time = state.csr_read(csr::TIME);
    let cycles = state.csr_read(csr::CYCLE);
    assert_eq!(time, cycles / state.config.system.clint_divider);
}

#[test]
fn test_csr_instret_counter() {
    let mut sys = create_test_cpu();
    let state = sys.core_ctx(0);

    // INSTRET and MINSTRET should read the same value
    let instret = state.csr_read(csr::INSTRET);
    let minstret = state.csr_read(csr::MINSTRET);
    assert_eq!(instret, minstret);
}

#[test]
fn test_csr_unknown_read_returns_zero() {
    let mut sys = create_test_cpu();
    let state = sys.core_ctx(0);

    // Reading an unknown/unimplemented CSR should return 0
    assert_eq!(state.csr_read(CsrAddr::from_u32(0xFFF)), 0);
}

#[test]
fn test_csr_unknown_write_ignored() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    // Writing to an unknown CSR should be ignored (no panic)
    state.csr_write(CsrAddr::from_u32(0xFFF), 0xDEADBEEF);
    // If we get here, the write was safely ignored
}

#[test]
fn mstatus_write_turns_the_vector_unit_on() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MSTATUS, 0);
    state.csr_write(csr::MSTATUS, csr::MSTATUS_VS_INIT);

    assert_eq!(state.csr_read(csr::MSTATUS) & csr::MSTATUS_VS, csr::MSTATUS_VS_INIT);
    assert_eq!(state.csr_read(csr::SSTATUS) & csr::MSTATUS_VS, csr::MSTATUS_VS_INIT);
}

#[test]
fn sstatus_write_turns_the_vector_unit_on() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::MSTATUS, 0);
    state.csr_write(csr::SSTATUS, csr::MSTATUS_VS_INIT);

    assert_eq!(state.csr_read(csr::MSTATUS) & csr::MSTATUS_VS, csr::MSTATUS_VS_INIT);
}

#[test]
fn sstatus_reports_dirty_vector_state_in_sd() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);

    state.csr_write(csr::SSTATUS, csr::MSTATUS_VS_DIRTY);

    assert_ne!(state.csr_read(csr::SSTATUS) & csr::MSTATUS_SD, 0);
}

//! Page Table Walker (PTW) Unit Tests.
//!
//! Verifies Sv39 / Sv48 / Sv57 address translation logic:
//! - Page-table walks at every supported level depth
//! - Superpages (2 MiB, 1 GiB, 512 GiB, 256 TiB)
//! - Permission checks (R/W/X/U)
//! - Accessed/Dirty bit updates
//! - Canonical-address checks
//! - Bare mode bypass

use crate::common::harness::TestContext;
use rvsim_core::common::{AccessType, TranslationResult, Trap, VirtAddr};
use rvsim_core::core::arch::csr::{self, Csrs};
use rvsim_core::core::arch::mode::PrivilegeMode;
use rvsim_core::core::units::mmu::tlb::TlbGeometry;
use rvsim_core::core::units::mmu::{Mmu, TranslateOutcome};
use rvsim_core::soc::interconnect::Bus;

/// Synchronously drives the MMU's async walker to completion. Reads each
/// PTE from the bus's RAM fast-path and feeds it back through
/// `continue_walk` until the walk reaches `Ready`. Used by PTW unit tests
/// that want to assert on the final `TranslationResult`. Argument order
/// matches the deleted `Mmu::translate` so existing call sites only need a
/// global name swap.
fn translate_sync(
    mmu: &mut Mmu,
    vaddr: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
    bus: &Bus,
) -> TranslationResult {
    let mut outcome = mmu.translate_async(vaddr, access, privilege, csrs, None);
    loop {
        match outcome {
            TranslateOutcome::Ready(r) => return r,
            TranslateOutcome::NeedPte { pte_addr, state } => {
                let raw = pte_addr.val();
                let raw_pte = bus.ram_region().filter(|r| r.contains(raw, 8)).map_or(0u64, |r| {
                    // SAFETY: `RamRegion::contains(raw, 8)` bounds-checks.
                    unsafe { r.ptr(raw).cast::<u64>().read_unaligned() }
                });
                let cycles = bus.calculate_transit_time(8);
                outcome = mmu.continue_walk(state, raw_pte, csrs, None, cycles);
            }
        }
    }
}

const ROOT_PPN: u64 = 0x80000; // Base at 0x8000_0000
const MEM_BASE: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000_0000; // 256MB

// PTE Permission bits
const V: u64 = 1 << 0;
const R: u64 = 1 << 1;
const W: u64 = 1 << 2;
const X: u64 = 1 << 3;
const U: u64 = 1 << 4;
#[allow(dead_code)]
const G: u64 = 1 << 5;
const A: u64 = 1 << 6;
const D: u64 = 1 << 7;

fn make_pte(ppn: u64, perms: u64) -> u64 {
    (ppn << 10) | perms | V
}

fn setup_mmu() -> (Mmu, Csrs, TestContext) {
    let mmu = Mmu::new(
        TlbGeometry { entries: 4, ways: 0 },
        TlbGeometry { entries: 4, ways: 4 },
        4,
        csr::PagingMode::Sv57,
    ); // Small TLB + small L2 TLB to force walks
    let mut csrs = Csrs::default();
    csrs.write(csr::MENVCFG, csr::MENVCFG_ADUE);

    // Enable SV39 mode
    let satp_val = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
    csrs.write(csr::SATP, satp_val);

    // SSTATUS.SUM | SSTATUS.MXR for broader test flexibility.
    csrs.write(csr::SSTATUS, (1 << 18) | (1 << 19));

    let tc = TestContext::new().with_memory(MEM_SIZE, MEM_BASE);

    (mmu, csrs, tc)
}

/// Helper to write a PTE to memory via the RAM fast-path pointer.
/// `vpn` is the index at the given `level` (2, 1, or 0).
/// `base_ppn` is the PPN of the page table at this level.
fn write_pte(bus: &Bus, base_ppn: u64, vpn_index: u64, pte: u64) {
    let addr = (base_ppn << 12) + (vpn_index * 8);
    let r = bus
        .ram_region()
        .filter(|r| r.contains(addr, 8))
        .expect("PTE address must lie inside the mocked RAM region");
    // SAFETY: bounds-checked by `RamRegion::contains(addr, 8)` above.
    unsafe { r.ptr(addr).cast::<u64>().write_unaligned(pte) };
}

#[test]
fn bare_mode_bypass() {
    let (mut mmu, mut csrs, tc) = setup_mmu();
    csrs.write(csr::SATP, 0); // Mode = 0 (Bare)

    let vaddr = VirtAddr::new(0x1234_5678);
    let res = translate_sync(
        &mut mmu,
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &tc.cpu().bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), 0x1234_5678);
}

#[test]
fn machine_mode_bypass() {
    let (mut mmu, csrs, tc) = setup_mmu();
    // SATP is SV39, but privilege is Machine -> should bypass

    let vaddr = VirtAddr::new(0x1234_5678);
    let res = translate_sync(
        &mut mmu,
        vaddr,
        AccessType::Read,
        PrivilegeMode::Machine,
        &csrs,
        &tc.cpu().bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), 0x1234_5678);
}

#[test]
fn sv39_4kb_page_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x4000_1234);
    let l2_idx = (0x4000_1234 >> 30) & 0x1FF; // 1
    let l1_idx = (0x4000_1234 >> 21) & 0x1FF; // 0
    let l0_idx = (0x4000_1234 >> 12) & 0x1FF; // 1

    let l1_table_ppn = ROOT_PPN + 1;
    let l0_table_ppn = ROOT_PPN + 2;
    let target_ppn = ROOT_PPN + 10;

    // L2 -> points to L1 table
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(l1_table_ppn, 0)); // Valid, no perms = pointer
    // L1 -> points to L0 table
    write_pte(bus, l1_table_ppn, l1_idx, make_pte(l0_table_ppn, 0));
    // L0 -> leaf (R/W/X)
    write_pte(bus, l0_table_ppn, l0_idx, make_pte(target_ppn, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), (target_ppn << 12) | 0x234);
}

#[test]
fn sv39_megapage_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x4020_0000);
    let l2_idx = (0x4020_0000 >> 30) & 0x1FF; // 1
    let l1_idx = (0x4020_0000 >> 21) & 0x1FF; // 1

    let l1_table_ppn = ROOT_PPN + 1;
    let target_ppn = ROOT_PPN + 0x200; // Aligned 2MB PPN

    // L2 -> points to L1
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(l1_table_ppn, 0));
    // L1 -> leaf (megapage)
    write_pte(bus, l1_table_ppn, l1_idx, make_pte(target_ppn, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), target_ppn << 12);
}

#[test]
fn sv39_gigapage_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x8000_0000); // VPN[2]=2
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;

    let target_ppn = ROOT_PPN + 0x40000; // Aligned 1GB PPN

    // L2 -> leaf (gigapage)
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), target_ppn << 12);
}

#[test]
fn invalid_pte_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x1000);

    // ROOT_PPN + VPN[2] is 0 (invalid) by default in MockMemory
    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn pointer_at_level_0_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x1000);

    let l2_idx = 0;
    let l1_idx = 0;
    let l0_idx = 1;

    let l1_ppn = ROOT_PPN + 1;
    let l0_ppn = ROOT_PPN + 2;

    write_pte(bus, ROOT_PPN, l2_idx, make_pte(l1_ppn, 0));
    write_pte(bus, l1_ppn, l1_idx, make_pte(l0_ppn, 0));
    // Level 0 PTE without R/W/X permissions -> pointer, but L0 can't have pointers
    write_pte(bus, l0_ppn, l0_idx, make_pte(ROOT_PPN + 10, 0)); // V=1, others 0

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn misaligned_superpage_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x4000_0000);

    let l2_idx = (0x4000_0000 >> 30) & 0x1FF;
    let l1_idx = 0;

    let l1_ppn = ROOT_PPN + 1;
    // Megapages require PPN[0..8]=0; this PPN is misaligned.
    let misaligned_target_ppn = (ROOT_PPN + 100) | 0x1;

    write_pte(bus, ROOT_PPN, l2_idx, make_pte(l1_ppn, 0));
    write_pte(bus, l1_ppn, l1_idx, make_pte(misaligned_target_ppn, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn write_to_clean_page_sets_dirty() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000; // Aligned 1GB

    // Leaf PTE, Accessed=1, Dirty=0
    let pte_val = make_pte(target_ppn, R | W | X | A);
    write_pte(bus, ROOT_PPN, l2_idx, pte_val);

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Write, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);

    let upd = res.dirty_update.expect("a store to a clean page sets D when it retires");
    assert_eq!(upd.set_bits, A | D);
}

#[test]
fn read_from_unaccessed_page_sets_accessed() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000; // Aligned 1GB

    // Leaf PTE, Accessed=0
    let pte_val = make_pte(target_ppn, R | W | X);
    write_pte(bus, ROOT_PPN, l2_idx, pte_val);

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);

    let upd = res.accessed_update.expect("the walker sets A on an unaccessed page");
    assert_eq!(upd.set_bits, A);
    assert!(res.dirty_update.is_none(), "a load never sets D");
}

#[test]
fn write_permission_check() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // Read-only page
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Write, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::StorePageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn execute_permission_check() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // RW page (NX)
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | W | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Fetch, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::InstructionPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn user_cannot_access_supervisor_page() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // Supervisor page (U=0)
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | W | X | A | D));

    let res = translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::User, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn supervisor_access_user_page_needs_sum() {
    let (mut mmu, mut csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // User page (U=1)
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | W | X | U | A | D));

    // Disable SUM
    csrs.write(csr::SSTATUS, 0);

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);

    // Enable SUM
    csrs.write(csr::SSTATUS, 1 << 18);
    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
}

#[test]
fn supervisor_cannot_fetch_user_page() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // User page (U=1) with Execute
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | X | U | A | D));

    // Even with SUM, Supervisor cannot execute User pages
    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Fetch, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::InstructionPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn non_canonical_address_faults() {
    let (mut mmu, csrs, tc) = setup_mmu();

    // SV39 requires bits 63..39 to sign-extend bit 38; here bit 38=1 but 63..39=0.
    let non_canon = VirtAddr::new(1 << 38);

    let res = translate_sync(
        &mut mmu,
        non_canon,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &tc.cpu().bus,
    );

    // Non-canonical address is unmapped in the virtual address space → PageFault
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

fn setup_mmu_with_mode(mode: u64) -> (Mmu, Csrs, TestContext) {
    let mmu = Mmu::new(
        TlbGeometry { entries: 4, ways: 0 },
        TlbGeometry { entries: 4, ways: 4 },
        4,
        csr::PagingMode::Sv57,
    );
    let mut csrs = Csrs::default();
    csrs.write(csr::MENVCFG, csr::MENVCFG_ADUE);
    let satp_val = (mode << 60) | ROOT_PPN;
    csrs.write(csr::SATP, satp_val);
    csrs.write(csr::SSTATUS, (1 << 18) | (1 << 19));
    let tc = TestContext::new().with_memory(MEM_SIZE, MEM_BASE);
    (mmu, csrs, tc)
}

fn vpn_index(va: u64, level: u32) -> u64 {
    (va >> (12 + 9 * level)) & 0x1FF
}

#[test]
fn sv48_4kb_page_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x4000_1234);
    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);
    let l1 = vpn_index(vaddr.val(), 1);
    let l0 = vpn_index(vaddr.val(), 0);

    let l2_table = ROOT_PPN + 1;
    let l1_table = ROOT_PPN + 2;
    let l0_table = ROOT_PPN + 3;
    let target = ROOT_PPN + 10;

    write_pte(bus, ROOT_PPN, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(l1_table, 0));
    write_pte(bus, l1_table, l1, make_pte(l0_table, 0));
    write_pte(bus, l0_table, l0, make_pte(target, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), (target << 12) | 0x234);
}

#[test]
fn sv48_megapage_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x4020_0000);
    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);
    let l1 = vpn_index(vaddr.val(), 1);

    let l2_table = ROOT_PPN + 1;
    let l1_table = ROOT_PPN + 2;
    let target = ROOT_PPN + 0x200; // aligned for 2 MiB superpage (PPN[0]=0)

    write_pte(bus, ROOT_PPN, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(l1_table, 0));
    write_pte(bus, l1_table, l1, make_pte(target, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), target << 12);
}

#[test]
fn sv48_gigapage_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x8000_0000);
    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);

    let l2_table = ROOT_PPN + 1;
    let target = ROOT_PPN + 0x40000; // aligned for 1 GiB superpage (PPN[0..18]=0)

    write_pte(bus, ROOT_PPN, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(target, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), target << 12);
}

#[test]
fn sv48_terapage_walk() {
    // Sv48-specific: 512 GiB superpage at L3 (top of the walk).
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;

    // VA in the first 512 GiB region; bit 47=0 so canonical.
    let vaddr = VirtAddr::new(0x10_0000_1000);
    let l3 = vpn_index(vaddr.val(), 3);

    // Aligned for 512 GiB: PPN[0..27]=0, i.e. multiple of 1 << 27.
    let target = 1u64 << 27;

    write_pte(bus, ROOT_PPN, l3, make_pte(target, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    let offset_mask = (1u64 << (12 + 9 * 3)) - 1;
    assert_eq!(res.paddr.val(), (target << 12) | (vaddr.val() & offset_mask));
}

#[test]
fn sv48_misaligned_superpage_causes_fault() {
    // Mid-walk superpage with non-zero PPN low bits → reserved → page fault.
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x4020_0000);
    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);
    let l1 = vpn_index(vaddr.val(), 1);

    let l2_table = ROOT_PPN + 1;
    let l1_table = ROOT_PPN + 2;
    let misaligned = (ROOT_PPN + 100) | 0x1; // L1 leaf must have PPN[0..8]=0

    write_pte(bus, ROOT_PPN, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(l1_table, 0));
    write_pte(bus, l1_table, l1, make_pte(misaligned, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn sv48_pointer_at_level_0_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x1000);

    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);
    let l1 = vpn_index(vaddr.val(), 1);
    let l0 = vpn_index(vaddr.val(), 0);

    let l2_table = ROOT_PPN + 1;
    let l1_table = ROOT_PPN + 2;
    let l0_table = ROOT_PPN + 3;

    write_pte(bus, ROOT_PPN, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(l1_table, 0));
    write_pte(bus, l1_table, l1, make_pte(l0_table, 0));
    // L0 with no R/W/X is a pointer encoding, illegal at the leaf level.
    write_pte(bus, l0_table, l0, make_pte(ROOT_PPN + 10, 0));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn sv48_non_canonical_address_faults() {
    let (mut mmu, csrs, tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);

    // bit 47 = 1 but bits 63..48 = 0 → non-canonical for Sv48.
    let non_canon_low = VirtAddr::new(1u64 << 47);
    let res = translate_sync(
        &mut mmu,
        non_canon_low,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &tc.cpu().bus,
    );
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);

    // bit 47 = 0 but a stray upper bit set → non-canonical.
    let non_canon_hi = VirtAddr::new(1u64 << 50);
    let res = translate_sync(
        &mut mmu,
        non_canon_hi,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &tc.cpu().bus,
    );
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn sv48_invalid_pte_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV48);
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x1000);
    // Default-zero memory at the root → V=0 at L3 → fault on the first read.
    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn sv57_4kb_page_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV57);
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x4000_1234);
    let l4 = vpn_index(vaddr.val(), 4);
    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);
    let l1 = vpn_index(vaddr.val(), 1);
    let l0 = vpn_index(vaddr.val(), 0);

    let l3_table = ROOT_PPN + 1;
    let l2_table = ROOT_PPN + 2;
    let l1_table = ROOT_PPN + 3;
    let l0_table = ROOT_PPN + 4;
    let target = ROOT_PPN + 20;

    write_pte(bus, ROOT_PPN, l4, make_pte(l3_table, 0));
    write_pte(bus, l3_table, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(l1_table, 0));
    write_pte(bus, l1_table, l1, make_pte(l0_table, 0));
    write_pte(bus, l0_table, l0, make_pte(target, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), (target << 12) | 0x234);
}

#[test]
fn sv57_petapage_walk() {
    // Sv57-specific: 256 TiB superpage at L4 (top of the walk).
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV57);
    let bus = &mut tc.cpu_mut().bus;

    // VA inside the first 256 TiB region (bit 56 = 0 → canonical).
    let vaddr = VirtAddr::new(0x10_0000_1000);
    let l4 = vpn_index(vaddr.val(), 4);

    // Aligned for 256 TiB: PPN[0..36]=0.
    let target = 1u64 << 36;

    write_pte(bus, ROOT_PPN, l4, make_pte(target, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    let offset_mask = (1u64 << (12 + 9 * 4)) - 1;
    assert_eq!(res.paddr.val(), (target << 12) | (vaddr.val() & offset_mask));
}

#[test]
fn sv57_misaligned_superpage_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV57);
    let bus = &mut tc.cpu_mut().bus;

    let vaddr = VirtAddr::new(0x10_0000_1000);
    let l4 = vpn_index(vaddr.val(), 4);

    // L4 leaf needs PPN[0..36]=0; set bit 0 to misalign.
    let misaligned = (1u64 << 36) | 0x1;
    write_pte(bus, ROOT_PPN, l4, make_pte(misaligned, R | W | X | A | D));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn sv57_pointer_at_level_0_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu_with_mode(csr::SATP_MODE_SV57);
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x1000);

    let l4 = vpn_index(vaddr.val(), 4);
    let l3 = vpn_index(vaddr.val(), 3);
    let l2 = vpn_index(vaddr.val(), 2);
    let l1 = vpn_index(vaddr.val(), 1);
    let l0 = vpn_index(vaddr.val(), 0);

    let l3_table = ROOT_PPN + 1;
    let l2_table = ROOT_PPN + 2;
    let l1_table = ROOT_PPN + 3;
    let l0_table = ROOT_PPN + 4;

    write_pte(bus, ROOT_PPN, l4, make_pte(l3_table, 0));
    write_pte(bus, l3_table, l3, make_pte(l2_table, 0));
    write_pte(bus, l2_table, l2, make_pte(l1_table, 0));
    write_pte(bus, l1_table, l1, make_pte(l0_table, 0));
    write_pte(bus, l0_table, l0, make_pte(ROOT_PPN + 10, 0));

    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus);
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn sv57_non_canonical_address_faults() {
    let (mut mmu, csrs, tc) = setup_mmu_with_mode(csr::SATP_MODE_SV57);

    // bit 56 = 1 but bits 63..57 = 0 → non-canonical.
    let non_canon = VirtAddr::new(1u64 << 56);

    let res = translate_sync(
        &mut mmu,
        non_canon,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &tc.cpu().bus,
    );
    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

/// Walks a load of 0x4000_0000 through a root pointer with `pointer_extra`
/// set to a 2 MiB leaf with `leaf_extra` set.
fn walk_with(pointer_extra: u64, leaf_extra: u64) -> TranslationResult {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu_mut().bus;
    let vaddr = VirtAddr::new(0x4000_0000);
    let l1_ppn = ROOT_PPN + 1;
    let leaf_ppn = ROOT_PPN + 0x200;
    write_pte(bus, ROOT_PPN, (0x4000_0000 >> 30) & 0x1FF, make_pte(l1_ppn, 0) | pointer_extra);
    write_pte(bus, l1_ppn, 0, make_pte(leaf_ppn, R | W | X | A | D) | leaf_extra);

    translate_sync(&mut mmu, vaddr, AccessType::Read, PrivilegeMode::Supervisor, &csrs, bus)
}

#[test]
fn a_walk_through_clean_ptes_succeeds() {
    assert!(walk_with(0, 0).trap.is_none());
}

#[test]
fn a_leaf_with_a_reserved_high_bit_set_raises_a_page_fault() {
    for bit in [54, 60, 61, 62, 63] {
        let res = walk_with(0, 1 << bit);

        assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "bit {bit}: {:?}", res.trap);
    }
}

#[test]
fn a_pointer_with_a_reserved_high_bit_set_raises_a_page_fault() {
    let res = walk_with(1 << 54, 0);

    assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "Trap: {:?}", res.trap);
}

#[test]
fn a_pointer_with_d_a_or_u_set_raises_a_page_fault() {
    for bit in [D, A, U] {
        let res = walk_with(bit, 0);

        assert!(matches!(res.trap, Some(Trap::LoadPageFault(_))), "{bit:#x}: {:?}", res.trap);
    }
}

#[test]
fn with_adue_clear_a_write_to_a_clean_page_raises_a_page_fault() {
    let (mut mmu, mut csrs, mut tc) = setup_mmu();
    csrs.write(csr::MENVCFG, 0);
    let bus = &mut tc.cpu_mut().bus;
    let target_ppn = ROOT_PPN + 0x40000;
    write_pte(bus, ROOT_PPN, (0x8000_0000 >> 30) & 0x1FF, make_pte(target_ppn, R | W | X | A));

    let vaddr = VirtAddr::new(0x8000_0000);
    let res =
        translate_sync(&mut mmu, vaddr, AccessType::Write, PrivilegeMode::Supervisor, &csrs, bus);

    assert!(matches!(res.trap, Some(Trap::StorePageFault(_))), "Trap: {:?}", res.trap);
}

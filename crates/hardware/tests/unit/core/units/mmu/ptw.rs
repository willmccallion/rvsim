//! Page Table Walker (PTW) Unit Tests.
//!
//! Verifies SV39 address translation logic:
//! - Page table walks (levels 2, 1, 0)
//! - Superpages (2MB, 1GB)
//! - Permission checks (R/W/X/U)
//! - Accessed/Dirty bit updates
//! - Canonical address checks
//! - Bare mode bypass

use crate::common::harness::TestContext;
use riscv_core::common::{AccessType, Trap, VirtAddr};
use riscv_core::core::arch::csr::{self, Csrs};
use riscv_core::core::arch::mode::PrivilegeMode;
use riscv_core::core::units::mmu::Mmu;
use riscv_core::soc::interconnect::Bus;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

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
    let mmu = Mmu::new(4); // Small TLB to force walks
    let mut csrs = Csrs::default();

    // Enable SV39 mode
    let satp_val = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
    csrs.write(csr::SATP, satp_val);

    // Enable SUM and MXR in sstatus for broader testing flexibility
    // SSTATUS_SUM (bit 18) = 1, SSTATUS_MXR (bit 19) = 1
    csrs.write(csr::SSTATUS, (1 << 18) | (1 << 19));

    let tc = TestContext::new().with_memory(MEM_SIZE, MEM_BASE);

    (mmu, csrs, tc)
}

/// Helper to write a PTE to memory.
/// `vpn` is the index at the given `level` (2, 1, or 0).
/// `base_ppn` is the PPN of the page table at this level.
fn write_pte(bus: &mut Bus, base_ppn: u64, vpn_index: u64, pte: u64) {
    let addr = (base_ppn << 12) + (vpn_index * 8);
    bus.write_u64(addr, pte);
}

// ══════════════════════════════════════════════════════════
// 1. Bare Mode
// ══════════════════════════════════════════════════════════

#[test]
fn bare_mode_bypass() {
    let (mut mmu, mut csrs, mut tc) = setup_mmu();
    csrs.write(csr::SATP, 0); // Mode = 0 (Bare)

    let vaddr = VirtAddr::new(0x1234_5678);
    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &mut tc.cpu.bus.bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), 0x1234_5678);
}

#[test]
fn machine_mode_bypass() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    // SATP is SV39, but privilege is Machine -> should bypass

    let vaddr = VirtAddr::new(0x1234_5678);
    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Machine,
        &csrs,
        &mut tc.cpu.bus.bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), 0x1234_5678);
}

// ══════════════════════════════════════════════════════════
// 2. 4KB Page Walk (3 Levels)
// ══════════════════════════════════════════════════════════

#[test]
fn sv39_4kb_page_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;

    // VA = 0x4000_1234
    // VPN[2] = 1, VPN[1] = 0, VPN[0] = 0
    // Offset = 0x234
    let vaddr = VirtAddr::new(0x4000_1234); // Binary: 0100 0000 0000 0000 0001 0010 0011 0100
    // VPN2=1, VPN1=0, VPN0=1, Off=0x234

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
    write_pte(
        bus,
        l0_table_ppn,
        l0_idx,
        make_pte(target_ppn, R | W | X | A | D),
    );

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), (target_ppn << 12) | 0x234);
}

// ══════════════════════════════════════════════════════════
// 3. Superpages (Megapage and Gigapage)
// ══════════════════════════════════════════════════════════

#[test]
fn sv39_megapage_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;

    // VA = 0x4020_0000
    // VPN[2]=1, VPN[1]=1
    let vaddr = VirtAddr::new(0x4020_0000);
    let l2_idx = (0x4020_0000 >> 30) & 0x1FF; // 1
    let l1_idx = (0x4020_0000 >> 21) & 0x1FF; // 1

    let l1_table_ppn = ROOT_PPN + 1;
    let target_ppn = ROOT_PPN + 0x200; // Aligned 2MB PPN

    // L2 -> points to L1
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(l1_table_ppn, 0));
    // L1 -> leaf (megapage)
    write_pte(
        bus,
        l1_table_ppn,
        l1_idx,
        make_pte(target_ppn, R | W | X | A | D),
    );

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), target_ppn << 12);
}

#[test]
fn sv39_gigapage_walk() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;

    let vaddr = VirtAddr::new(0x8000_0000); // VPN[2]=2
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;

    let target_ppn = ROOT_PPN + 0x40000; // Aligned 1GB PPN

    // L2 -> leaf (gigapage)
    write_pte(
        bus,
        ROOT_PPN,
        l2_idx,
        make_pte(target_ppn, R | W | X | A | D),
    );

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
    assert_eq!(res.paddr.val(), target_ppn << 12);
}

// ══════════════════════════════════════════════════════════
// 4. Invalid and Malformed PTEs
// ══════════════════════════════════════════════════════════

#[test]
fn invalid_pte_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x1000);

    // ROOT_PPN + VPN[2] is 0 (invalid) by default in MockMemory
    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );

    assert!(
        matches!(res.trap, Some(Trap::LoadPageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

#[test]
fn pointer_at_level_0_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
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

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(
        matches!(res.trap, Some(Trap::LoadPageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

#[test]
fn misaligned_superpage_causes_fault() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x4000_0000);

    let l2_idx = (0x4000_0000 >> 30) & 0x1FF;
    let l1_idx = 0;

    let l1_ppn = ROOT_PPN + 1;
    // PPN must be aligned for megapage (PPN[0..8] must be 0)
    // We set a misaligned PPN
    let misaligned_target_ppn = (ROOT_PPN + 100) | 0x1;

    write_pte(bus, ROOT_PPN, l2_idx, make_pte(l1_ppn, 0));
    write_pte(
        bus,
        l1_ppn,
        l1_idx,
        make_pte(misaligned_target_ppn, R | W | X | A | D),
    );

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(
        matches!(res.trap, Some(Trap::LoadPageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

// ══════════════════════════════════════════════════════════
// 5. Access Permissions & A/D Bits
// ══════════════════════════════════════════════════════════

#[test]
fn write_to_clean_page_sets_dirty() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000; // Aligned 1GB

    // Leaf PTE, Accessed=1, Dirty=0
    let pte_val = make_pte(target_ppn, R | W | X | A);
    write_pte(bus, ROOT_PPN, l2_idx, pte_val);

    let res = mmu.translate(
        vaddr,
        AccessType::Write,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);

    // Check if Dirty bit was updated in memory
    let new_pte = bus.read_u64(ROOT_PPN << 12 | (l2_idx * 8));
    assert_eq!(new_pte & D, D, "Dirty bit should be set");
}

#[test]
fn read_from_unaccessed_page_sets_accessed() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000; // Aligned 1GB

    // Leaf PTE, Accessed=0
    let pte_val = make_pte(target_ppn, R | W | X);
    write_pte(bus, ROOT_PPN, l2_idx, pte_val);

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );

    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);

    // Check if Accessed bit was updated in memory
    let new_pte = bus.read_u64(ROOT_PPN << 12 | (l2_idx * 8));
    assert_eq!(new_pte & A, A, "Accessed bit should be set");
}

#[test]
fn write_permission_check() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // Read-only page
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | A | D));

    let res = mmu.translate(
        vaddr,
        AccessType::Write,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(
        matches!(res.trap, Some(Trap::StorePageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

#[test]
fn execute_permission_check() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // RW page (NX)
    write_pte(bus, ROOT_PPN, l2_idx, make_pte(target_ppn, R | W | A | D));

    let res = mmu.translate(
        vaddr,
        AccessType::Fetch,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(
        matches!(res.trap, Some(Trap::InstructionPageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

// ══════════════════════════════════════════════════════════
// 6. User / Supervisor Checks
// ══════════════════════════════════════════════════════════

#[test]
fn user_cannot_access_supervisor_page() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // Supervisor page (U=0)
    write_pte(
        bus,
        ROOT_PPN,
        l2_idx,
        make_pte(target_ppn, R | W | X | A | D),
    );

    let res = mmu.translate(vaddr, AccessType::Read, PrivilegeMode::User, &csrs, bus);
    assert!(
        matches!(res.trap, Some(Trap::LoadPageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

#[test]
fn supervisor_access_user_page_needs_sum() {
    let (mut mmu, mut csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // User page (U=1)
    write_pte(
        bus,
        ROOT_PPN,
        l2_idx,
        make_pte(target_ppn, R | W | X | U | A | D),
    );

    // Disable SUM
    csrs.write(csr::SSTATUS, 0);

    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(
        matches!(res.trap, Some(Trap::LoadPageFault(_))),
        "Trap: {:?}",
        res.trap
    );

    // Enable SUM
    csrs.write(csr::SSTATUS, 1 << 18);
    let res = mmu.translate(
        vaddr,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(res.trap.is_none(), "Trap: {:?}", res.trap);
}

#[test]
fn supervisor_cannot_fetch_user_page() {
    let (mut mmu, csrs, mut tc) = setup_mmu();
    let bus = &mut tc.cpu.bus.bus;
    let vaddr = VirtAddr::new(0x8000_0000);
    let l2_idx = (0x8000_0000 >> 30) & 0x1FF;
    let target_ppn = ROOT_PPN + 0x40000;

    // User page (U=1) with Execute
    write_pte(
        bus,
        ROOT_PPN,
        l2_idx,
        make_pte(target_ppn, R | X | U | A | D),
    );

    // Even with SUM, Supervisor cannot execute User pages
    let res = mmu.translate(
        vaddr,
        AccessType::Fetch,
        PrivilegeMode::Supervisor,
        &csrs,
        bus,
    );
    assert!(
        matches!(res.trap, Some(Trap::InstructionPageFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

// ══════════════════════════════════════════════════════════
// 7. Canonical Address Check
// ══════════════════════════════════════════════════════════

#[test]
fn non_canonical_address_faults() {
    let (mut mmu, csrs, mut tc) = setup_mmu();

    // SV39 addresses must have bits 63:38 all equal (sign extension of bit 38)
    // 0x0000_0040_0000_0000 -> bit 38 is 1, so 63:39 must be 1.
    // Here 63:39 are 0, so it's non-canonical.
    // 39 bits = 512GB space. Top bit is bit 38.

    // Construct a non-canonical address.
    // Bit 38 is 1, so bits 63..39 should be 1.
    let non_canon = VirtAddr::new(1 << 38);

    let res = mmu.translate(
        non_canon,
        AccessType::Read,
        PrivilegeMode::Supervisor,
        &csrs,
        &mut tc.cpu.bus.bus,
    );

    // Non-canonical access triggers AccessFault (not PageFault)
    assert!(
        matches!(res.trap, Some(Trap::LoadAccessFault(_))),
        "Trap: {:?}",
        res.trap
    );
}

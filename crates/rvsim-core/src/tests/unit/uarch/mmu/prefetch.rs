//! Where a load prefetch may go, as the data TLB already knows: the MMU
//! looks without walking or disturbing replacement, and refuses a page
//! the load could not read.

use crate::arch::csr::Csrs;
use crate::common::{Asid, PhysAddr, Ppn, VirtAddr, Vpn};
use crate::isa::csr;
use crate::isa::privileged::{PagingMode, PrivilegeMode};
use crate::uarch::mmu::tlb::{PageSize, TlbGeometry};
use crate::uarch::mmu::{Mmu, PrefetchTranslation};

const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;

const PAGE: u64 = 0x1234;
const FRAME: u64 = 0x8_0042;

fn mmu() -> Mmu {
    Mmu::new(
        TlbGeometry { entries: 8, ways: 0 },
        TlbGeometry { entries: 0, ways: 0 },
        4,
        PagingMode::Sv57,
    )
}

fn sv39() -> Csrs {
    let mut csrs = Csrs::default();
    csrs.write(csr::SATP, csr::SATP_MODE_SV39 << 60);
    csrs
}

/// An MMU whose data TLB maps virtual page `PAGE` to `FRAME` under `pte`.
fn mmu_mapping(pte: u64, size: PageSize) -> Mmu {
    let mut mmu = mmu();
    mmu.dtlb.insert(Vpn::new(PAGE), Ppn::new(FRAME), pte | PTE_V, Asid::new(0), size);
    mmu
}

fn in_page(offset: u64) -> VirtAddr {
    VirtAddr::new((PAGE << 12) | offset)
}

#[test]
fn machine_mode_and_bare_are_untranslated() {
    let mmu = mmu();

    let machine = mmu.prefetch_translation(in_page(0), PrivilegeMode::Machine, &sv39());
    let bare = mmu.prefetch_translation(in_page(0), PrivilegeMode::Supervisor, &Csrs::default());

    assert_eq!(
        (machine, bare),
        (PrefetchTranslation::Untranslated, PrefetchTranslation::Untranslated)
    );
}

#[test]
fn a_page_the_dtlb_holds_maps_with_its_size() {
    let mmu = mmu_mapping(PTE_R, PageSize::Kib4);

    let translation = mmu.prefetch_translation(in_page(0x40), PrivilegeMode::Supervisor, &sv39());

    assert_eq!(
        translation,
        PrefetchTranslation::Mapped {
            paddr: PhysAddr::new((FRAME << 12) | 0x40),
            page: PageSize::Kib4,
        }
    );
}

#[test]
fn a_page_the_dtlb_does_not_hold_is_missing() {
    let mmu = mmu();

    let translation = mmu.prefetch_translation(in_page(0), PrivilegeMode::Supervisor, &sv39());

    assert_eq!(translation, PrefetchTranslation::Missing);
}

#[test]
fn an_unreadable_page_is_denied() {
    let mmu = mmu_mapping(PTE_X, PageSize::Kib4);

    let translation = mmu.prefetch_translation(in_page(0), PrivilegeMode::Supervisor, &sv39());

    assert_eq!(translation, PrefetchTranslation::Denied);
}

#[test]
fn a_user_page_is_denied_to_supervisor_without_sum() {
    let mmu = mmu_mapping(PTE_R | PTE_U, PageSize::Kib4);
    let mut with_sum = sv39();
    with_sum.write(csr::SSTATUS, 1 << 18);

    let without = mmu.prefetch_translation(in_page(0), PrivilegeMode::Supervisor, &sv39());
    let with = mmu.prefetch_translation(in_page(0), PrivilegeMode::Supervisor, &with_sum);

    assert_eq!(without, PrefetchTranslation::Denied);
    assert!(matches!(with, PrefetchTranslation::Mapped { .. }));
}

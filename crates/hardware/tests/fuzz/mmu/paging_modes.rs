//! Property tests across Sv39 / Sv48 / Sv57 page-table walks.
//!
//! For each random `(mode, leaf_level, vpn_path, leaf_ppn)` we install a
//! page table that maps a chosen VA to a chosen PPN at the chosen level
//! (4KB / 2MB / 1GB / 512GB / 256TB depending on level), then call
//! `Mmu::translate` and check that the returned paddr matches the expected
//! `ppn.to_addr() | (va & offset_mask)`. Non-canonical VAs are also
//! exercised and must page-fault.

use crate::common::harness::TestContext;
use proptest::prelude::*;
use rvsim_core::common::{AccessType, PhysAddr, TranslationResult, Trap, VirtAddr};
use rvsim_core::core::arch::csr::{self, Csrs};
use rvsim_core::core::units::mmu::tlb::TlbGeometry;
use rvsim_core::core::units::mmu::{Mmu, TranslateOutcome};
use rvsim_core::isa::privileged::mode::PrivilegeMode;

/// Drives a synchronous translation: repeatedly invokes `translate_async` /
/// `continue_walk` against the MMU and reads PTEs out of memory through the
/// test context so the property tests stay shaped like the deleted
/// synchronous `Mmu::translate`.
fn translate_sync(
    mmu: &mut Mmu,
    va: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
    ctx: &mut TestContext,
) -> TranslationResult {
    let mut outcome = mmu.translate_async(va, access, privilege, csrs, None);
    loop {
        match outcome {
            TranslateOutcome::Ready(result) => return result,
            TranslateOutcome::NeedPte { pte_addr, state } => {
                let raw_pte = ctx.sim.probe_mem_load(pte_addr, 8);
                outcome = mmu.continue_walk(state, raw_pte, csrs, None, 0);
            }
        }
    }
}

const ROOT_PPN: u64 = 0x80000;
const MEM_BASE: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x0400_0000; // 64 MiB — enough for tables + leaf PPNs.

const PAGE_SHIFT: u32 = 12;
const VPN_BITS: u32 = 9;
const VPN_INDEX_MASK: u64 = 0x1FF;
const PTE_SIZE: u64 = 8;
const PTE_V: u64 = 1;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;
const PTE_PERMS_RW: u64 = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D;

fn make_pte(ppn: u64, perms: u64) -> u64 {
    (ppn << 10) | perms
}

fn write_pte(ctx: &mut TestContext, table_ppn: u64, vpn_index: u64, pte: u64) {
    let addr = (table_ppn << PAGE_SHIFT) | (vpn_index * PTE_SIZE);
    ctx.sim.probe_mem_store(PhysAddr::new(addr), pte, 8);
}

fn mode_to_satp(mode: u64) -> u64 {
    (mode << 60) | ROOT_PPN
}

fn modes() -> Vec<(u64, usize)> {
    vec![(csr::SATP_MODE_SV39, 3), (csr::SATP_MODE_SV48, 4), (csr::SATP_MODE_SV57, 5)]
}

fn build_mmu(satp: u64) -> (Mmu, Csrs, TestContext) {
    let mmu = Mmu::new(
        TlbGeometry { entries: 4, ways: 0 },
        TlbGeometry { entries: 4, ways: 4 },
        4,
        csr::PagingMode::Sv57,
    );
    let mut csrs = Csrs::default();
    csrs.write(csr::MENVCFG, csr::MENVCFG_ADUE);
    csrs.write(csr::SATP, satp);
    csrs.write(csr::SSTATUS, (1 << 18) | (1 << 19)); // SUM | MXR
    let ctx = TestContext::new().with_memory(MEM_SIZE, MEM_BASE);
    (mmu, csrs, ctx)
}

/// Builds a page-table chain mapping `va` to `leaf_ppn` at `leaf_level`.
/// Returns the expected paddr for `va`.
fn install_table(
    ctx: &mut TestContext,
    levels: usize,
    leaf_level: usize,
    va: u64,
    leaf_ppn: u64,
) -> u64 {
    let mut table_ppn = ROOT_PPN;
    for level in (leaf_level + 1..levels).rev() {
        let next_ppn = ROOT_PPN + (levels - level) as u64;
        let vpn_i = (va >> (PAGE_SHIFT + VPN_BITS * level as u32)) & VPN_INDEX_MASK;
        write_pte(ctx, table_ppn, vpn_i, make_pte(next_ppn, PTE_V));
        table_ppn = next_ppn;
    }
    let leaf_vpn_i = (va >> (PAGE_SHIFT + VPN_BITS * leaf_level as u32)) & VPN_INDEX_MASK;
    write_pte(ctx, table_ppn, leaf_vpn_i, make_pte(leaf_ppn, PTE_PERMS_RW));

    let offset_shift = PAGE_SHIFT + VPN_BITS * leaf_level as u32;
    let offset_mask = (1u64 << offset_shift) - 1;
    (leaf_ppn << PAGE_SHIFT) | (va & offset_mask)
}

/// Builds a canonical VA whose VPN path is `vpn_path` (highest level
/// first); page offset slots into the bits below `leaf_level`'s superpage.
fn build_va(levels: usize, leaf_level: usize, vpn_path: &[u64], page_offset: u64) -> u64 {
    let mut va: u64 = 0;
    for level in (0..levels).rev() {
        let bits = vpn_path[level] & VPN_INDEX_MASK;
        va |= bits << (PAGE_SHIFT + VPN_BITS * level as u32);
    }
    let offset_shift = PAGE_SHIFT + VPN_BITS * leaf_level as u32;
    let offset_mask = (1u64 << offset_shift) - 1;
    let va = (va & !offset_mask) | (page_offset & offset_mask);
    sign_extend_canonical(va, levels)
}

/// Sign-extends `va` from the mode's top VPN bit so it's canonical.
fn sign_extend_canonical(va: u64, levels: usize) -> u64 {
    let top_bit = (PAGE_SHIFT + VPN_BITS * levels as u32) as u64 - 1;
    let mask = (1u64 << (top_bit + 1)) - 1;
    let mut v = va & mask;
    if (v >> top_bit) & 1 == 1 {
        v |= !mask;
    }
    v
}

/// Picks an aligned leaf PPN well clear of `ROOT_PPN..` table region.
fn aligned_leaf_ppn(leaf_level: usize, raw: u64) -> u64 {
    let align_shift = (VPN_BITS * leaf_level as u32) as u64;
    let align = 1u64 << align_shift;
    let base = (ROOT_PPN + 0x100).next_multiple_of(align);
    let span = ((MEM_SIZE as u64 / 4096).saturating_sub(base)) / align;
    let span = span.max(1);
    base + (raw % span) * align
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Picking any supported mode and any allowed leaf level, an installed
    /// translation must round-trip through `Mmu::translate`.
    #[test]
    fn random_walks_round_trip(
        mode_idx in 0usize..3,
        leaf_level_seed in 0u8..5,
        vpn_path in proptest::collection::vec(0u64..0x1FF, 5),
        page_offset in 0u64..0xFFF_FFFF_FFFF_FFFF,
        leaf_ppn_seed in 0u64..1024,
    ) {
        let (mode_val, levels) = modes()[mode_idx];
        let leaf_level = leaf_level_seed as usize % levels;

        let (mut mmu, csrs, mut ctx) = build_mmu(mode_to_satp(mode_val));
        let va = build_va(levels, leaf_level, &vpn_path, page_offset);
        let leaf_ppn = aligned_leaf_ppn(leaf_level, leaf_ppn_seed);
        let expected_paddr = install_table(&mut ctx, levels, leaf_level, va, leaf_ppn);

        let result = translate_sync(
            &mut mmu,
            VirtAddr::new(va),
            AccessType::Read,
            PrivilegeMode::Supervisor,
            &csrs,
            &mut ctx,
        );
        prop_assert!(result.trap.is_none(), "unexpected trap: {:?}", result.trap);
        prop_assert_eq!(result.paddr.val(), expected_paddr);
    }

    /// A VA with a stray bit above the mode's top must page-fault before
    /// any walk happens.
    #[test]
    fn non_canonical_addresses_fault(
        mode_idx in 0usize..3,
        stray_bit in 0u32..7,
    ) {
        let (mode_val, levels) = modes()[mode_idx];
        let top_bit = PAGE_SHIFT + VPN_BITS * levels as u32 - 1;
        let bit = top_bit as u64 + 1 + stray_bit as u64;
        if bit >= 64 {
            return Ok(());
        }
        let va = 1u64 << bit;

        let (mut mmu, csrs, mut ctx) = build_mmu(mode_to_satp(mode_val));
        let result = translate_sync(
            &mut mmu,
            VirtAddr::new(va),
            AccessType::Read,
            PrivilegeMode::Supervisor,
            &csrs,
            &mut ctx,
        );
        prop_assert!(matches!(result.trap, Some(Trap::LoadPageFault(_))));
    }

    /// A leaf PPN with non-zero low bits below the leaf's superpage must
    /// fault as a misaligned-superpage page-fault.
    #[test]
    fn misaligned_superpage_faults(
        mode_idx in 0usize..3,
        leaf_level_seed in 1u8..5,
    ) {
        let (mode_val, levels) = modes()[mode_idx];
        let leaf_level = leaf_level_seed as usize % levels;
        if leaf_level == 0 {
            return Ok(());
        }

        let (mut mmu, csrs, mut ctx) = build_mmu(mode_to_satp(mode_val));
        let va = build_va(levels, leaf_level, &[0; 5], 0);
        let aligned_ppn = aligned_leaf_ppn(leaf_level, 1);
        let misaligned_ppn = aligned_ppn | 0x1;
        let _ = install_table(&mut ctx, levels, leaf_level, va, misaligned_ppn);

        let result = translate_sync(
            &mut mmu,
            VirtAddr::new(va),
            AccessType::Read,
            PrivilegeMode::Supervisor,
            &csrs,
            &mut ctx,
        );
        prop_assert!(matches!(result.trap, Some(Trap::LoadPageFault(_))));
    }
}

//! Hardware Page Table Walker (PTW) for RISC-V SV39.
//!
//! This module implements the hardware page table walking algorithm. It traverses
//! the three-level page table structure defined by the SV39 virtual memory scheme
//! to translate virtual addresses to physical addresses.

use crate::common::{
    AccessType, Asid, PAGE_SHIFT, PhysAddr, Ppn, TranslationResult, Trap, VPN_MASK, VirtAddr, Vpn,
};
use crate::core::arch::csr::{Csrs, SATP_ASID_MASK, SATP_ASID_SHIFT, SATP_PPN_MASK};
use crate::core::arch::mode::PrivilegeMode;
use crate::core::units::mmu::Mmu;
use crate::core::units::mmu::pmp::{Pmp, PmpResult};
use crate::soc::interconnect::Bus;

/// Page Table Entry valid bit (bit 0).
const PTE_VALID_BIT: u64 = 1;

/// Page Table Entry read permission bit (bit 1).
const PTE_READ_BIT: u64 = 1 << 1;

/// Page Table Entry write permission bit (bit 2).
const PTE_WRITE_BIT: u64 = 1 << 2;

/// Page Table Entry execute permission bit (bit 3).
const PTE_EXEC_BIT: u64 = 1 << 3;

/// Page Table Entry user mode access bit (bit 4).
const PTE_USER_BIT: u64 = 1 << 4;

/// Page Table Entry accessed bit (bit 6).
const PTE_ACCESSED_BIT: u64 = 1 << 6;

/// Page Table Entry dirty bit (bit 7).
const PTE_DIRTY_BIT: u64 = 1 << 7;

/// Bit shift to extract Physical Page Number from PTE (bits 10-53).
const PTE_PPN_SHIFT: u64 = 10;

/// A strongly-typed wrapper around a raw 64-bit SV39 Page Table Entry.
#[derive(Clone, Copy, Debug)]
struct PageTableEntry(u64);

impl PageTableEntry {
    /// Creates a new `PageTableEntry` from a raw 64-bit value.
    const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Returns the underlying raw 64-bit value.
    const fn raw(self) -> u64 {
        self.0
    }

    /// Returns true if the Valid (V) bit is set.
    const fn is_valid(self) -> bool {
        self.0 & PTE_VALID_BIT != 0
    }

    /// Returns true if the Read (R) bit is set.
    const fn can_read(self) -> bool {
        self.0 & PTE_READ_BIT != 0
    }

    /// Returns true if the Write (W) bit is set.
    const fn can_write(self) -> bool {
        self.0 & PTE_WRITE_BIT != 0
    }

    /// Returns true if the Execute (X) bit is set.
    const fn can_exec(self) -> bool {
        self.0 & PTE_EXEC_BIT != 0
    }

    /// Returns true if the User (U) bit is set.
    const fn is_user(self) -> bool {
        self.0 & PTE_USER_BIT != 0
    }

    /// Returns true if the Accessed (A) bit is set.
    const fn is_accessed(self) -> bool {
        self.0 & PTE_ACCESSED_BIT != 0
    }

    /// Returns true if the Dirty (D) bit is set.
    const fn is_dirty(self) -> bool {
        self.0 & PTE_DIRTY_BIT != 0
    }

    /// Extracts the Physical Page Number (PPN) from the entry.
    const fn ppn(self) -> Ppn {
        Ppn::new((self.0 >> PTE_PPN_SHIFT) & SATP_PPN_MASK)
    }

    /// Extracts the raw PPN value as u64 (for bitwise operations).
    const fn ppn_raw(self) -> u64 {
        (self.0 >> PTE_PPN_SHIFT) & SATP_PPN_MASK
    }

    /// Determines if this entry is a pointer to the next level page table.
    ///
    /// In SV39, an entry is a pointer if it is Valid but has R=0, W=0, and X=0.
    const fn is_pointer(self) -> bool {
        !self.can_read() && !self.can_write() && !self.can_exec()
    }

    /// Returns a new instance with the Accessed (A) bit set.
    const fn with_accessed(self) -> Self {
        Self(self.0 | PTE_ACCESSED_BIT)
    }

    /// Returns a new instance with the Dirty (D) bit set.
    const fn with_dirty(self) -> Self {
        Self(self.0 | PTE_DIRTY_BIT)
    }
}

/// Performs a hardware page table walk for SV39.
///
/// Traverses the page table tree starting from the root PPN in the SATP register.
/// It supports 4KB pages, 2MB megapages, and 1GB gigapages.
///
/// # Arguments
///
/// * `mmu` - Mutable reference to the MMU for TLB updates.
/// * `vaddr` - The virtual address to translate.
/// * `access` - The type of memory access (Fetch, Read, Write).
/// * `privilege` - The current privilege mode of the processor.
/// * `csrs` - System CSRs (specifically SATP and STATUS).
/// * `bus` - System bus for reading PTEs from memory.
pub fn page_table_walk(
    mmu: &mut Mmu,
    vaddr: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
    bus: &mut Bus,
) -> TranslationResult {
    page_table_walk_inner(mmu, vaddr, access, privilege, csrs, bus, None)
}

/// Page table walk with optional PMP checking on PTE reads.
pub fn page_table_walk_with_pmp(
    mmu: &mut Mmu,
    vaddr: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
    bus: &mut Bus,
    pmp: &Pmp,
) -> TranslationResult {
    page_table_walk_inner(mmu, vaddr, access, privilege, csrs, bus, Some(pmp))
}

fn page_table_walk_inner(
    mmu: &mut Mmu,
    vaddr: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
    bus: &mut Bus,
    pmp: Option<&Pmp>,
) -> TranslationResult {
    /// Number of page table levels in SV39 (3 levels: L2, L1, L0).
    const SV39_LEVELS: usize = 3;

    /// Number of bits used for VPN indexing at each level (9 bits per level).
    const VPN_BITS_PER_LEVEL: u64 = 9;

    /// Bit mask to extract VPN index from virtual address (9 bits: 0x1FF).
    const VPN_ENTRY_MASK: u64 = 0x1FF;

    /// Size of a Page Table Entry in bytes (8 bytes for 64-bit PTE).
    const PTE_SIZE: u64 = 8;

    /// Cycles required to update a PTE's accessed/dirty bits in memory.
    const PTE_UPDATE_CYCLES: u64 = 10;

    let satp = csrs.satp;
    let mut ppn_raw = satp & SATP_PPN_MASK;
    let asid = Asid::new(((satp >> SATP_ASID_SHIFT) & SATP_ASID_MASK) as u16);
    let mut cycles = 0;

    for level in (0..SV39_LEVELS).rev() {
        let vpn_shift = PAGE_SHIFT + level as u64 * VPN_BITS_PER_LEVEL;
        let vpn_i = (vaddr.val() >> vpn_shift) & VPN_ENTRY_MASK;
        let pte_addr = (ppn_raw << PAGE_SHIFT) + (vpn_i * PTE_SIZE);

        // PMP check on PTE read address (spec requires PMP enforcement on PTW accesses)
        if let Some(pmp_unit) = pmp {
            let pmp_result = pmp_unit.check(pte_addr, 8, true, false, false, false);
            if pmp_result != PmpResult::Allow {
                return TranslationResult::fault(
                    match access {
                        AccessType::Fetch => Trap::InstructionAccessFault(vaddr.val()),
                        AccessType::Read => Trap::LoadAccessFault(vaddr.val()),
                        AccessType::Write => Trap::StoreAccessFault(vaddr.val()),
                    },
                    cycles,
                );
            }
        }

        cycles += bus.calculate_transit_time(8);
        let raw_pte = bus.read_u64(crate::common::PhysAddr::new(pte_addr));
        let pte = PageTableEntry::new(raw_pte);

        if !pte.is_valid() {
            return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
        }

        if pte.is_pointer() {
            if level == 0 {
                return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
            }
            ppn_raw = pte.ppn_raw();
            continue;
        }

        // W=1, R=0 is a reserved PTE encoding (spec 4.3.1) — must fault.
        if pte.can_write() && !pte.can_read() {
            return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
        }

        if level > 0 {
            let ppn_mask = (1 << (level as u64 * VPN_BITS_PER_LEVEL)) - 1;
            if (pte.ppn_raw() & ppn_mask) != 0 {
                return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
            }
        }

        if check_permissions(pte, access, privilege, csrs).is_err() {
            return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
        }

        // Software-managed A/D bits: fault instead of auto-setting.
        // This matches spike's behavior where A=0 or D=0 triggers a
        // page fault so the OS trap handler can set the bits.
        if mmu.software_ad_bits {
            if !pte.is_accessed() {
                return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
            }
            if access == AccessType::Write && !pte.is_dirty() {
                return TranslationResult::fault(page_fault(vaddr.val(), access), cycles);
            }
        }

        let (new_pte, updated) = update_access_bits(pte, access);

        // Defer A/D bit writes to commit — the instruction may be
        // speculative and could be squashed. Writing A/D bits here
        // would corrupt kernel page-table state irreversibly.
        let pte_update = if updated {
            cycles += PTE_UPDATE_CYCLES;
            Some(crate::common::error::PteUpdate {
                pte_addr: crate::common::PhysAddr::new(pte_addr),
                pte_value: new_pte.raw(),
            })
        } else {
            None
        };

        let final_ppn = pte.ppn();

        let offset_mask = (1u64 << vpn_shift) - 1;
        let final_paddr = final_ppn.to_addr() | (vaddr.val() & offset_mask);

        let specific_4kb_ppn = Ppn::new(final_paddr >> PAGE_SHIFT);
        let vpn = Vpn::new((vaddr.val() >> PAGE_SHIFT) & VPN_MASK);

        // Insert the *original* PTE bits into TLBs (without speculative
        // A/D updates). After commit writes the A/D bits to RAM, the
        // next access will re-walk and cache the correct state.
        let pte_raw = pte.raw();
        if access == AccessType::Fetch {
            mmu.itlb.insert(vpn, specific_4kb_ppn, pte_raw, asid);
        } else {
            mmu.dtlb.insert(vpn, specific_4kb_ppn, pte_raw, asid);
        }
        // Also populate the shared L2 TLB.
        mmu.l2_tlb.insert(vpn, specific_4kb_ppn, pte_raw, asid);

        return pte_update.map_or_else(
            || TranslationResult::success(PhysAddr::new(final_paddr), cycles),
            |update| {
                TranslationResult::success_with_pte_update(
                    PhysAddr::new(final_paddr),
                    cycles,
                    update,
                )
            },
        );
    }

    TranslationResult::fault(page_fault(vaddr.val(), access), cycles)
}

/// Validates access permissions for a leaf PTE.
///
/// Checks R/W/X bits, User bit, and status register flags (MXR, SUM).
/// Returns `Ok(())` if access is allowed, `Err(())` otherwise.
fn check_permissions(
    pte: PageTableEntry,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
) -> Result<(), ()> {
    /// Bit position of MXR (Make eXecutable Readable) bit in sstatus register.
    const SSTATUS_MXR_SHIFT: u64 = 19;
    /// Bit position of SUM (Supervisor User Memory access) bit in sstatus register.
    const SSTATUS_SUM_SHIFT: u64 = 18;

    if access == AccessType::Write && !pte.can_write() {
        return Err(());
    }
    if access == AccessType::Fetch && !pte.can_exec() {
        return Err(());
    }

    let mxr = (csrs.sstatus >> SSTATUS_MXR_SHIFT) & 1 != 0;

    if access == AccessType::Read && !(pte.can_read() || (pte.can_exec() && mxr)) {
        return Err(());
    }

    if privilege == PrivilegeMode::User && !pte.is_user() {
        return Err(());
    }

    if privilege == PrivilegeMode::Supervisor && pte.is_user() {
        let sum = (csrs.sstatus >> SSTATUS_SUM_SHIFT) & 1 != 0;
        if !sum {
            return Err(());
        }
        if access == AccessType::Fetch {
            return Err(());
        }
    }

    Ok(())
}

/// Updates the Accessed (A) and Dirty (D) bits of a PTE.
///
/// Returns a tuple containing the potentially modified PTE and a boolean
/// indicating if a write-back to memory is required.
fn update_access_bits(pte: PageTableEntry, access: AccessType) -> (PageTableEntry, bool) {
    let need_accessed = !pte.is_accessed();
    let need_dirty = access == AccessType::Write && !pte.is_dirty();
    let updated = need_accessed || need_dirty;

    let new_pte = if need_accessed { pte.with_accessed() } else { pte };
    let new_pte = if need_dirty { new_pte.with_dirty() } else { new_pte };

    (new_pte, updated)
}

/// Constructs the appropriate Trap for a failed page access.
const fn page_fault(addr: u64, access: AccessType) -> Trap {
    match access {
        AccessType::Fetch => Trap::InstructionPageFault(addr),
        AccessType::Read => Trap::LoadPageFault(addr),
        AccessType::Write => Trap::StorePageFault(addr),
    }
}

//! Memory Management Unit (MMU).
//!
//! This module implements the Memory Management Unit, responsible for
//! virtual-to-physical address translation. It supports the RISC-V SV39
//! paging scheme and includes Translation Lookaside Buffers (TLBs) for
//! caching translations.

/// Physical Memory Protection (PMP).
pub mod pmp;

/// Page table walker implementation for SV39 virtual memory.
pub mod ptw;

/// Translation Lookaside Buffer (TLB) for caching virtual-to-physical address translations.
pub mod tlb;

use crate::common::{AccessType, PhysAddr, TranslationResult, Trap, VirtAddr};
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::soc::interconnect::Bus;

use self::tlb::Tlb;

/// Memory Management Unit (MMU) for virtual-to-physical address translation.
///
/// Implements RISC-V SV39 page-based virtual memory with separate instruction
/// and data translation lookaside buffers (TLBs) and page table walker.
pub struct Mmu {
    /// Data TLB for load/store address translation.
    pub dtlb: Tlb,
    /// Instruction TLB for fetch address translation.
    pub itlb: Tlb,
}

impl Mmu {
    /// Creates a new MMU with the specified TLB size.
    ///
    /// # Arguments
    ///
    /// * `tlb_size` - Number of entries in each TLB (instruction and data)
    ///
    /// # Returns
    ///
    /// A new `Mmu` instance with initialized TLBs.
    pub fn new(tlb_size: usize) -> Self {
        Self {
            dtlb: Tlb::new(tlb_size),
            itlb: Tlb::new(tlb_size),
        }
    }

    /// Translates a virtual address to a physical address.
    ///
    /// Performs address translation using the page table walker and TLBs,
    /// checking permissions and handling page faults. Supports SV39 paging
    /// and bare mode (no translation).
    ///
    /// # Arguments
    ///
    /// * `vaddr` - Virtual address to translate
    /// * `access` - Type of access (Fetch, Read, Write)
    /// * `privilege` - Current privilege mode
    /// * `csrs` - Control and status registers (for SATP, SSTATUS)
    /// * `bus` - System bus for page table walks
    ///
    /// # Returns
    ///
    /// A `TranslationResult` containing the physical address, cycle count,
    /// and any trap that occurred during translation.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use hardware::core::units::mmu::Mmu;
    /// use hardware::common::{VirtAddr, AccessType};
    /// use hardware::core::arch::mode::PrivilegeMode;
    ///
    /// let mut mmu = Mmu::new(32);
    ///
    /// // In machine mode, addresses pass through without translation
    /// let result = mmu.translate(
    ///     VirtAddr::new(0x80000000),
    ///     AccessType::Fetch,
    ///     PrivilegeMode::Machine,
    ///     &csrs,
    ///     &mut bus,
    /// );
    /// assert_eq!(result.paddr.val(), 0x80000000);
    /// assert!(result.trap.is_none());
    ///
    /// // In supervisor mode with paging enabled, TLB is consulted
    /// // and page table walk is performed on TLB miss
    /// ```
    pub fn translate(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        privilege: PrivilegeMode,
        csrs: &Csrs,
        bus: &mut Bus,
    ) -> TranslationResult {
        let satp = csrs.satp;
        use crate::core::arch::csr::{
            SATP_MODE_BARE, SATP_MODE_MASK, SATP_MODE_SHIFT, SATP_MODE_SV39,
        };
        let mode = (satp >> SATP_MODE_SHIFT) & SATP_MODE_MASK;

        if privilege == PrivilegeMode::Machine || mode == SATP_MODE_BARE {
            return TranslationResult::success(PhysAddr::new(vaddr.val()), 0);
        }

        if mode != SATP_MODE_SV39 {
            return TranslationResult::fault(Trap::InstructionAccessFault(vaddr.val()), 0);
        }

        let va = vaddr.val();
        let bit_38 = (va >> 38) & 1;
        let top_bits = va >> 39;
        let expected_top = if bit_38 == 1 { 0x1FFFFFF } else { 0 };
        if top_bits != expected_top {
            return TranslationResult::fault(
                match access {
                    AccessType::Fetch => Trap::InstructionAccessFault(va),
                    AccessType::Read => Trap::LoadAccessFault(va),
                    AccessType::Write => Trap::StoreAccessFault(va),
                },
                0,
            );
        }

        use crate::common::constants::{PAGE_SHIFT, VPN_MASK};
        let vpn = (vaddr.val() >> PAGE_SHIFT) & VPN_MASK;

        let tlb_entry = if access == AccessType::Fetch {
            self.itlb.lookup(vpn)
        } else {
            self.dtlb.lookup(vpn)
        };

        if let Some((ppn, r, w, x, u)) = tlb_entry {
            if access == AccessType::Write && !w {
                return TranslationResult::fault(Trap::StorePageFault(vaddr.val()), 0);
            }
            if access == AccessType::Fetch && !x {
                return TranslationResult::fault(Trap::InstructionPageFault(vaddr.val()), 0);
            }
            if access == AccessType::Read {
                /// Bit position of MXR (Make eXecutable Readable) bit in sstatus register.
                const SSTATUS_MXR_SHIFT: u64 = 19;
                let mxr = (csrs.sstatus >> SSTATUS_MXR_SHIFT) & 1 != 0;
                let readable = r || (x && mxr);
                if !readable {
                    return TranslationResult::fault(Trap::LoadPageFault(vaddr.val()), 0);
                }
            }

            if privilege == PrivilegeMode::User && !u {
                return TranslationResult::fault(page_fault(vaddr.val(), access), 0);
            }
            if privilege == PrivilegeMode::Supervisor && u {
                /// Bit position of SUM (Supervisor User Memory access) bit in sstatus register.
                const SSTATUS_SUM_SHIFT: u64 = 18;
                let sum = (csrs.sstatus >> SSTATUS_SUM_SHIFT) & 1 != 0;
                if !sum {
                    return TranslationResult::fault(page_fault(vaddr.val(), access), 0);
                }
                if access == AccessType::Fetch {
                    return TranslationResult::fault(Trap::InstructionPageFault(vaddr.val()), 0);
                }
            }

            use crate::common::constants::PAGE_SHIFT;
            let paddr = (ppn << PAGE_SHIFT) | vaddr.page_offset();
            return TranslationResult::success(PhysAddr::new(paddr), 0);
        }

        ptw::page_table_walk(self, vaddr, access, privilege, csrs, bus)
    }
}

/// Creates an appropriate page fault trap for the access type.
///
/// # Arguments
///
/// * `addr` - The faulting virtual address
/// * `access` - The type of access that caused the fault
///
/// # Returns
///
/// The appropriate `Trap` variant for the page fault.
fn page_fault(addr: u64, access: AccessType) -> Trap {
    match access {
        AccessType::Fetch => Trap::InstructionPageFault(addr),
        AccessType::Read => Trap::LoadPageFault(addr),
        AccessType::Write => Trap::StorePageFault(addr),
    }
}

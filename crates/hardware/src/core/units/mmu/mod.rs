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

use crate::common::{AccessType, Asid, PhysAddr, TranslationResult, Trap, VirtAddr, Vpn};
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::units::mmu::pmp::Pmp;
use crate::soc::interconnect::Bus;

use self::tlb::{L2Tlb, Tlb};

/// Memory Management Unit (MMU) for virtual-to-physical address translation.
///
/// Implements RISC-V SV39 page-based virtual memory with separate instruction
/// and data L1 TLBs, a shared L2 TLB, and a page table walker.
#[derive(Debug)]
pub struct Mmu {
    /// Data TLB for load/store address translation.
    pub dtlb: Tlb,
    /// Instruction TLB for fetch address translation.
    pub itlb: Tlb,
    /// Shared L2 TLB (set-associative, consulted on L1 miss).
    pub l2_tlb: L2Tlb,
    /// Software-managed A/D bits: PTW faults on A=0 or D=0 instead of
    /// auto-setting them (matches spike's behavior).
    pub software_ad_bits: bool,
}

impl Mmu {
    /// Creates a new MMU with the specified TLB sizes.
    ///
    /// # Arguments
    ///
    /// * `tlb_size` - Number of entries in each L1 TLB (instruction and data)
    /// * `l2_size` - Total number of entries in the shared L2 TLB
    /// * `l2_ways` - L2 TLB associativity (ways per set)
    /// * `l2_latency` - L2 TLB hit latency in cycles
    ///
    /// # Returns
    ///
    /// A new `Mmu` instance with initialized TLBs.
    pub fn new(
        tlb_size: usize,
        l2_size: usize,
        l2_ways: usize,
        l2_latency: u64,
        software_ad_bits: bool,
    ) -> Self {
        Self {
            dtlb: Tlb::new(tlb_size),
            itlb: Tlb::new(tlb_size),
            l2_tlb: L2Tlb::new(l2_size, l2_ways, l2_latency),
            software_ad_bits,
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
        self.translate_with_pmp(vaddr, access, privilege, csrs, bus, None)
    }

    /// Translates a virtual address with optional PMP enforcement.
    ///
    /// Same as [`translate`](Self::translate) but also checks PMP permissions when `pmp` is `Some`.
    pub fn translate_with_pmp(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        privilege: PrivilegeMode,
        csrs: &Csrs,
        bus: &mut Bus,
        pmp: Option<&Pmp>,
    ) -> TranslationResult {
        use crate::common::constants::{PAGE_SHIFT, VPN_MASK};
        use crate::core::arch::csr::{
            SATP_ASID_MASK, SATP_ASID_SHIFT, SATP_MODE_BARE, SATP_MODE_MASK, SATP_MODE_SHIFT,
            SATP_MODE_SV39,
        };
        /// Bit position of MXR (Make eXecutable Readable) bit in sstatus register.
        const SSTATUS_MXR_SHIFT: u64 = 19;
        /// Bit position of SUM (Supervisor User Memory access) bit in sstatus register.
        const SSTATUS_SUM_SHIFT: u64 = 18;

        let satp = csrs.satp;
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
                    AccessType::Fetch => Trap::InstructionPageFault(va),
                    AccessType::Read => Trap::LoadPageFault(va),
                    AccessType::Write => Trap::StorePageFault(va),
                },
                0,
            );
        }
        let vpn = Vpn::new((vaddr.val() >> PAGE_SHIFT) & VPN_MASK);
        let asid = Asid::new(((satp >> SATP_ASID_SHIFT) & SATP_ASID_MASK) as u16);

        let tlb_entry = if access == AccessType::Fetch {
            self.itlb.lookup(vpn, asid)
        } else {
            self.dtlb.lookup(vpn, asid)
        };

        if let Some(hit) = tlb_entry {
            // If writing to a page with D=0, invalidate the TLB entry and
            // fall through to the page table walk so the PTW sets the dirty
            // bit in the PTE and re-caches with D=1.
            if access == AccessType::Write && !hit.d {
                self.dtlb.invalidate(vpn);
            } else {
                if access == AccessType::Write && !hit.w {
                    return TranslationResult::fault(Trap::StorePageFault(vaddr.val()), 0);
                }
                if access == AccessType::Fetch && !hit.x {
                    return TranslationResult::fault(Trap::InstructionPageFault(vaddr.val()), 0);
                }
                if access == AccessType::Read {
                    let mxr = (csrs.sstatus >> SSTATUS_MXR_SHIFT) & 1 != 0;
                    let readable = hit.r || (hit.x && mxr);
                    if !readable {
                        return TranslationResult::fault(Trap::LoadPageFault(vaddr.val()), 0);
                    }
                }

                if privilege == PrivilegeMode::User && !hit.u {
                    return TranslationResult::fault(page_fault(vaddr.val(), access), 0);
                }
                if privilege == PrivilegeMode::Supervisor && hit.u {
                    let sum = (csrs.sstatus >> SSTATUS_SUM_SHIFT) & 1 != 0;
                    if !sum {
                        return TranslationResult::fault(page_fault(vaddr.val(), access), 0);
                    }
                    if access == AccessType::Fetch {
                        return TranslationResult::fault(
                            Trap::InstructionPageFault(vaddr.val()),
                            0,
                        );
                    }
                }

                let paddr = hit.ppn.to_addr() | vaddr.page_offset();
                return TranslationResult::success(PhysAddr::new(paddr), 0);
            }
        }

        // L1 TLB miss — check the shared L2 TLB before invoking the PTW.
        let l2_latency = self.l2_tlb.latency;
        if let Some((ppn, pte_bits, entry_asid)) = self.l2_tlb.lookup(vpn, asid) {
            // Extract permission bits from cached PTE.
            let r = (pte_bits >> 1) & 1 != 0;
            let w = (pte_bits >> 2) & 1 != 0;
            let x = (pte_bits >> 3) & 1 != 0;
            let u = (pte_bits >> 4) & 1 != 0;
            let d = (pte_bits >> 7) & 1 != 0;

            // Check dirty bit — if writing with D=0, fall through to PTW.
            if access == AccessType::Write && !d {
                // Don't return; let the PTW set the dirty bit.
                // Do NOT promote to L1 — the entry has D=0 and would
                // cause a repeated fallthrough on the next L1 hit.
            } else {
                // Permission checks (must mirror the L1 TLB hit path).
                // Run these BEFORE promoting to L1 so faulting entries
                // never pollute the L1 cache.
                if access == AccessType::Write && !w {
                    return TranslationResult::fault(Trap::StorePageFault(vaddr.val()), l2_latency);
                }
                if access == AccessType::Fetch && !x {
                    return TranslationResult::fault(
                        Trap::InstructionPageFault(vaddr.val()),
                        l2_latency,
                    );
                }
                if access == AccessType::Read {
                    let mxr = (csrs.sstatus >> SSTATUS_MXR_SHIFT) & 1 != 0;
                    if !(r || (x && mxr)) {
                        return TranslationResult::fault(
                            Trap::LoadPageFault(vaddr.val()),
                            l2_latency,
                        );
                    }
                }

                if privilege == PrivilegeMode::User && !u {
                    return TranslationResult::fault(page_fault(vaddr.val(), access), l2_latency);
                }
                if privilege == PrivilegeMode::Supervisor && u {
                    let sum = (csrs.sstatus >> SSTATUS_SUM_SHIFT) & 1 != 0;
                    if !sum {
                        return TranslationResult::fault(
                            page_fault(vaddr.val(), access),
                            l2_latency,
                        );
                    }
                    if access == AccessType::Fetch {
                        return TranslationResult::fault(
                            Trap::InstructionPageFault(vaddr.val()),
                            l2_latency,
                        );
                    }
                }

                // Permissions passed — promote to L1 TLB.
                if access == AccessType::Fetch {
                    self.itlb.insert(vpn, ppn, pte_bits, entry_asid);
                } else {
                    self.dtlb.insert(vpn, ppn, pte_bits, entry_asid);
                }

                let paddr = ppn.to_addr() | vaddr.page_offset();
                return TranslationResult::success(PhysAddr::new(paddr), l2_latency);
            }
        }

        if let Some(pmp_unit) = pmp {
            ptw::page_table_walk_with_pmp(self, vaddr, access, privilege, csrs, bus, pmp_unit)
        } else {
            ptw::page_table_walk(self, vaddr, access, privilege, csrs, bus)
        }
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
const fn page_fault(addr: u64, access: AccessType) -> Trap {
    match access {
        AccessType::Fetch => Trap::InstructionPageFault(addr),
        AccessType::Read => Trap::LoadPageFault(addr),
        AccessType::Write => Trap::StorePageFault(addr),
    }
}

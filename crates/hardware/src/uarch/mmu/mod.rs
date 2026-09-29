//! Memory Management Unit (MMU).
//!
//! Handles RISC-V virtual-to-physical translation for the Sv39 / Sv48 / Sv57
//! paging modes. The MMU owns the TLB hierarchy (per-access L1 TLB, shared
//! L2 TLB) and exposes [`Mmu::translate_async`] — an event-driven API that
//! either resolves the translation immediately or returns the first PTE
//! address the caller must read via a `MemReq` packet.

pub mod ptw;

pub mod tlb;

use crate::arch::csr::Csrs;
use crate::arch::pmp::Pmp;
use crate::arch::translation::{SfenceVmaInfo, TranslationResult};
use crate::common::{AccessType, Asid, PAGE_SHIFT, PhysAddr, VPN_MASK, VirtAddr, Vpn};
use crate::isa::privileged::PagingMode;
use crate::isa::privileged::{PrivilegeMode, Trap};

use self::ptw::{WalkRequest, WalkState, WalkStep};
use self::tlb::{Tlb, TlbGeometry, TlbHit};

/// Outcome of [`Mmu::translate_async`].
///
/// `Ready` means translation completed without needing memory; `NeedPte`
/// means the caller must issue a `MemReq` for `pte_addr`, stash `state`
/// in its outstanding-walks table, and call [`Mmu::continue_walk`] with the
/// 64-bit PTE value when the response arrives.
#[derive(Clone, Debug)]
pub enum TranslateOutcome {
    /// Translation completed (success, fault, direct mode, TLB hit).
    Ready(TranslationResult),
    /// Walk needs to read a PTE from memory.
    NeedPte {
        /// Physical address of the PTE to fetch.
        pte_addr: PhysAddr,
        /// Walk state to stash until the response arrives.
        state: WalkState,
    },
}

/// Memory Management Unit (MMU) for virtual-to-physical address translation.
///
/// Implements RISC-V SV39 / Sv48 / Sv57 page-based virtual memory with
/// separate instruction and data L1 TLBs, a shared L2 TLB, and a stateful
/// page-table walker that emits `MemReq` packets through the event queue.
#[derive(Debug)]
pub struct Mmu {
    /// Data TLB for load/store address translation.
    pub dtlb: Tlb,
    /// Instruction TLB for fetch address translation.
    pub itlb: Tlb,
    /// Shared L2 TLB, consulted on an L1 miss; empty when not configured.
    pub l2_tlb: Tlb,
    /// L2 TLB hit latency in cycles.
    pub l2_tlb_latency: u64,
    /// Highest SATP paging mode the CPU writer will accept. Anything above
    /// this is coerced to Bare on write, letting tests pin a mode without
    /// rebuilding the kernel (e.g. force a Sv57-aware Linux onto Sv39).
    pub paging_mode_max: PagingMode,
}

impl Mmu {
    /// Retires an SFENCE.VMA: flushes the translations it names from every
    /// TLB. The caller also clears the hart's reservation.
    pub fn sfence_vma(&mut self, info: &SfenceVmaInfo) {
        let vpn = || Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
        let asid = || Asid::new(info.rs2_val as u16);
        for tlb in [&mut self.dtlb, &mut self.itlb, &mut self.l2_tlb] {
            match (!info.rs1_idx.is_zero(), !info.rs2_idx.is_zero()) {
                (false, false) => tlb.flush(),
                (true, false) => tlb.flush_vaddr(vpn()),
                (false, true) => tlb.flush_asid(asid()),
                (true, true) => tlb.flush_vaddr_asid(vpn(), asid()),
            }
        }
    }

    /// Creates an MMU with instruction and data L1 TLBs of `l1`'s geometry,
    /// a shared L2 TLB of `l2`'s hitting after `l2_latency` cycles, and a
    /// SATP writer accepting modes up to `paging_mode_max`.
    #[must_use]
    pub fn new(
        l1: TlbGeometry,
        l2: TlbGeometry,
        l2_latency: u64,
        paging_mode_max: PagingMode,
    ) -> Self {
        Self {
            dtlb: Tlb::new(l1),
            itlb: Tlb::new(l1),
            l2_tlb: Tlb::new(l2),
            l2_tlb_latency: l2_latency,
            paging_mode_max,
        }
    }

    /// Attempts to translate `vaddr`.
    ///
    /// Resolves direct-mode, M-mode, Bare, canonical-VA, TLB-hit, and
    /// L2-TLB-hit cases immediately and returns
    /// [`TranslateOutcome::Ready`]. On a TLB miss the walker is started
    /// and [`TranslateOutcome::NeedPte`] is returned: the caller issues a
    /// `MemReq` for `pte_addr`, stashes `state`, and resumes the walk via
    /// [`Mmu::continue_walk`] when the PTE response arrives.
    pub fn translate_async(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        privilege: PrivilegeMode,
        csrs: &Csrs,
        pmp: Option<&Pmp>,
    ) -> TranslateOutcome {
        use crate::common::{PAGE_SHIFT, VPN_MASK};
        use crate::isa::csr::{
            MSTATUS_MXR, MSTATUS_SUM, SATP_ASID_MASK, SATP_ASID_SHIFT, SATP_MODE_MASK,
            SATP_MODE_SHIFT,
        };
        use crate::isa::privileged::PagingMode;

        let satp = csrs.satp;
        let mode_raw = (satp >> SATP_MODE_SHIFT) & SATP_MODE_MASK;
        let Some(paging) = PagingMode::from_satp_mode(mode_raw) else {
            return TranslateOutcome::Ready(TranslationResult::fault(
                Trap::InstructionAccessFault(vaddr.val()),
                0,
            ));
        };

        if privilege == PrivilegeMode::Machine || paging == PagingMode::Bare {
            return TranslateOutcome::Ready(TranslationResult::success(
                PhysAddr::new(vaddr.val()),
                0,
            ));
        }

        let va = vaddr.val();
        if !is_canonical_va(va, paging) {
            return TranslateOutcome::Ready(TranslationResult::fault(
                match access {
                    AccessType::Fetch => Trap::InstructionPageFault(va),
                    AccessType::Read => Trap::LoadPageFault(va),
                    AccessType::Write => Trap::StorePageFault(va),
                },
                0,
            ));
        }
        let vpn = Vpn::new((vaddr.val() >> PAGE_SHIFT) & VPN_MASK);
        let asid = Asid::new(((satp >> SATP_ASID_SHIFT) & SATP_ASID_MASK) as u16);

        let tlb_entry = if access == AccessType::Fetch {
            self.itlb.lookup(vpn, asid)
        } else {
            self.dtlb.lookup(vpn, asid)
        };

        if let Some(hit) = tlb_entry {
            if access == AccessType::Write && !hit.d {
                self.dtlb.invalidate(vpn);
            } else {
                if access == AccessType::Write && !hit.w {
                    return TranslateOutcome::Ready(TranslationResult::fault(
                        Trap::StorePageFault(vaddr.val()),
                        0,
                    ));
                }
                if access == AccessType::Fetch && !hit.x {
                    return TranslateOutcome::Ready(TranslationResult::fault(
                        Trap::InstructionPageFault(vaddr.val()),
                        0,
                    ));
                }
                if access == AccessType::Read {
                    let mxr = csrs.mstatus & MSTATUS_MXR != 0;
                    let readable = hit.r || (hit.x && mxr);
                    if !readable {
                        return TranslateOutcome::Ready(TranslationResult::fault(
                            Trap::LoadPageFault(vaddr.val()),
                            0,
                        ));
                    }
                }

                if privilege == PrivilegeMode::User && !hit.u {
                    return TranslateOutcome::Ready(TranslationResult::fault(
                        page_fault(vaddr.val(), access),
                        0,
                    ));
                }
                if privilege == PrivilegeMode::Supervisor && hit.u {
                    let sum = csrs.mstatus & MSTATUS_SUM != 0;
                    if !sum {
                        return TranslateOutcome::Ready(TranslationResult::fault(
                            page_fault(vaddr.val(), access),
                            0,
                        ));
                    }
                    if access == AccessType::Fetch {
                        return TranslateOutcome::Ready(TranslationResult::fault(
                            Trap::InstructionPageFault(vaddr.val()),
                            0,
                        ));
                    }
                }

                let paddr = hit.ppn.to_addr() | vaddr.page_offset();
                return TranslateOutcome::Ready(TranslationResult::success(
                    PhysAddr::new(paddr),
                    0,
                ));
            }
        }

        let l2_latency = self.l2_tlb_latency;
        if let Some(hit) = self.l2_tlb.lookup(vpn, asid) {
            let TlbHit { ppn, r, w, x, u, d, mapping } = hit;

            if access == AccessType::Write && !d {
                // fall through to PTW so it sets the dirty bit
            } else {
                if access == AccessType::Write && !w {
                    return TranslateOutcome::Ready(TranslationResult::fault(
                        Trap::StorePageFault(vaddr.val()),
                        l2_latency,
                    ));
                }
                if access == AccessType::Fetch && !x {
                    return TranslateOutcome::Ready(TranslationResult::fault(
                        Trap::InstructionPageFault(vaddr.val()),
                        l2_latency,
                    ));
                }
                if access == AccessType::Read {
                    let mxr = csrs.mstatus & MSTATUS_MXR != 0;
                    if !(r || (x && mxr)) {
                        return TranslateOutcome::Ready(TranslationResult::fault(
                            Trap::LoadPageFault(vaddr.val()),
                            l2_latency,
                        ));
                    }
                }

                if privilege == PrivilegeMode::User && !u {
                    return TranslateOutcome::Ready(TranslationResult::fault(
                        page_fault(vaddr.val(), access),
                        l2_latency,
                    ));
                }
                if privilege == PrivilegeMode::Supervisor && u {
                    let sum = csrs.mstatus & MSTATUS_SUM != 0;
                    if !sum {
                        return TranslateOutcome::Ready(TranslationResult::fault(
                            page_fault(vaddr.val(), access),
                            l2_latency,
                        ));
                    }
                    if access == AccessType::Fetch {
                        return TranslateOutcome::Ready(TranslationResult::fault(
                            Trap::InstructionPageFault(vaddr.val()),
                            l2_latency,
                        ));
                    }
                }

                if access == AccessType::Fetch {
                    self.itlb.insert_mapping(mapping);
                } else {
                    self.dtlb.insert_mapping(mapping);
                }

                let paddr = ppn.to_addr() | vaddr.page_offset();
                return TranslateOutcome::Ready(TranslationResult::success(
                    PhysAddr::new(paddr),
                    l2_latency,
                ));
            }
        }

        let request = WalkRequest { access, privilege, mode: paging };
        match ptw::start_walk(request, vaddr, csrs, pmp) {
            WalkStep::Done(result) => TranslateOutcome::Ready(result),
            WalkStep::NeedPte { pte_addr, state } => TranslateOutcome::NeedPte { pte_addr, state },
        }
    }

    /// Continues an in-flight walk after the caller has loaded the PTE
    /// at `state.pte_addr` from memory.
    pub fn continue_walk(
        &mut self,
        state: WalkState,
        raw_pte: u64,
        csrs: &Csrs,
        pmp: Option<&Pmp>,
        bus_transit_cycles: u64,
    ) -> TranslateOutcome {
        match ptw::continue_walk(state, raw_pte, self, csrs, pmp, bus_transit_cycles) {
            WalkStep::Done(result) => TranslateOutcome::Ready(result),
            WalkStep::NeedPte { pte_addr, state } => TranslateOutcome::NeedPte { pte_addr, state },
        }
    }
}

/// Returns true if `va` is a canonical virtual address for `mode`.
const fn is_canonical_va(va: u64, mode: crate::isa::privileged::PagingMode) -> bool {
    let top = mode.va_top_bit();
    if top >= 63 {
        return true;
    }
    let top_bit = (va >> top) & 1;
    let upper = va >> (top + 1);
    let expected = if top_bit == 1 { (1u64 << (63 - top)) - 1 } else { 0 };
    upper == expected
}

/// Creates an appropriate page fault trap for the access type.
const fn page_fault(addr: u64, access: AccessType) -> Trap {
    match access {
        AccessType::Fetch => Trap::InstructionPageFault(addr),
        AccessType::Read => Trap::LoadPageFault(addr),
        AccessType::Write => Trap::StorePageFault(addr),
    }
}

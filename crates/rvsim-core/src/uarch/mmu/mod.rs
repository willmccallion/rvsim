//! Memory Management Unit (MMU).
//!
//! Handles RISC-V virtual-to-physical translation for the Sv39 / Sv48 / Sv57
//! paging modes. The MMU owns the TLB hierarchy (per-access L1 TLB, shared
//! L2 TLB) and exposes [`Mmu::translate_async`] — an event-driven API that
//! either resolves the translation immediately or returns the first PTE
//! address the caller must read via a `MemReq` packet.

pub mod perms;

pub mod ptw;

pub mod tlb;

use crate::arch::csr::Csrs;
use crate::arch::pmp::Pmp;
use crate::arch::translation::{SfenceVmaInfo, TranslationResult};
use crate::common::{AccessType, Asid, PhysAddr, VirtAddr};
use crate::isa::privileged::{PagingMode, Satp};
use crate::isa::privileged::{PrivilegeMode, Trap};

use self::ptw::{WalkRequest, WalkState, WalkStep};
use self::tlb::{PageSize, Tlb, TlbGeometry, TlbHit};

/// Where a hardware prefetch to a virtual address may go, as the data TLB
/// already knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefetchTranslation {
    /// No translation applies (M-mode or Bare): the address is physical.
    Untranslated,
    /// The data TLB maps the page to `paddr` in a page of `page`'s size.
    Mapped {
        /// The physical address.
        paddr: PhysAddr,
        /// The size of the page holding it.
        page: PageSize,
    },
    /// The data TLB does not hold the page.
    Missing,
    /// The page may not be read at this privilege.
    Denied,
}

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

/// An access looking up its page in a TLB.
struct HitLookup {
    vaddr: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    mstatus: u64,
}

impl HitLookup {
    /// What `hit` gives the access: its address, or the page fault its
    /// permissions raise; `None` for a write to a clean page, which walks
    /// so the walker sets D. The walk's fill replaces the entry for that
    /// page, so the clean one is left in place until then.
    fn outcome(&self, hit: &TlbHit) -> Option<Result<PhysAddr, Trap>> {
        if self.access == AccessType::Write && !hit.d {
            return None;
        }
        if !hit.perms().permits(self.access, self.privilege, self.mstatus) {
            return Some(Err(Trap::page_fault(self.access, self.vaddr.val())));
        }
        Some(Ok(PhysAddr::new(hit.ppn.to_addr() | self.vaddr.page_offset())))
    }
}

impl Mmu {
    /// Retires an SFENCE.VMA: flushes the translations it names from every
    /// TLB. The caller also clears the hart's reservation.
    pub fn sfence_vma(&mut self, info: &SfenceVmaInfo) {
        let vpn = || VirtAddr::new(info.rs1_val).page_number();
        let asid = || Asid::from_register(info.rs2_val);
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
        let satp = Satp::new(csrs.satp);
        let Some(paging) = satp.paging_mode() else {
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
                Trap::page_fault(access, va),
                0,
            ));
        }
        let vpn = vaddr.page_number();
        let asid = satp.asid();

        let l1_hit = if access == AccessType::Fetch {
            self.itlb.lookup(vpn, asid)
        } else {
            self.dtlb.lookup(vpn, asid)
        };
        let lookup = HitLookup { vaddr, access, privilege, mstatus: csrs.mstatus };
        match l1_hit.and_then(|hit| lookup.outcome(&hit)) {
            Some(Ok(paddr)) => {
                return TranslateOutcome::Ready(TranslationResult::success(paddr, 0));
            }
            Some(Err(trap)) => return TranslateOutcome::Ready(TranslationResult::fault(trap, 0)),
            None => {}
        }

        let l2_latency = self.l2_tlb_latency;
        if let Some(hit) = self.l2_tlb.lookup(vpn, asid) {
            match lookup.outcome(&hit) {
                Some(Ok(paddr)) => {
                    if access == AccessType::Fetch {
                        self.itlb.insert_mapping(hit.mapping);
                    } else {
                        self.dtlb.insert_mapping(hit.mapping);
                    }
                    return TranslateOutcome::Ready(TranslationResult::success(paddr, l2_latency));
                }
                Some(Err(trap)) => {
                    return TranslateOutcome::Ready(TranslationResult::fault(trap, l2_latency));
                }
                None => {}
            }
        }

        let request = WalkRequest { access, privilege, mode: paging };
        match ptw::start_walk(request, vaddr, csrs, pmp) {
            WalkStep::Done(result) => TranslateOutcome::Ready(result),
            WalkStep::NeedPte { pte_addr, state } => TranslateOutcome::NeedPte { pte_addr, state },
        }
    }

    /// Where a load prefetch to `vaddr` at `privilege` may go: the data TLB
    /// is consulted without touching its replacement state, nothing walks,
    /// and a page the load could not read is refused.
    #[must_use]
    pub fn prefetch_translation(
        &self,
        vaddr: VirtAddr,
        privilege: PrivilegeMode,
        csrs: &Csrs,
    ) -> PrefetchTranslation {
        let satp = Satp::new(csrs.satp);
        let Some(paging) = satp.paging_mode() else {
            return PrefetchTranslation::Denied;
        };
        if privilege == PrivilegeMode::Machine || paging == PagingMode::Bare {
            return PrefetchTranslation::Untranslated;
        }
        if !is_canonical_va(vaddr.val(), paging) {
            return PrefetchTranslation::Denied;
        }
        let vpn = vaddr.page_number();
        let asid = satp.asid();
        let Some(hit) = self.dtlb.peek(vpn, asid) else { return PrefetchTranslation::Missing };
        if !hit.perms().permits(AccessType::Read, privilege, csrs.mstatus) {
            return PrefetchTranslation::Denied;
        }
        PrefetchTranslation::Mapped {
            paddr: PhysAddr::new(hit.ppn.to_addr() | vaddr.page_offset()),
            page: hit.mapping.size(),
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

/// Translates `vaddr` for a data `access` at `privilege` now, reading each
/// PTE with `read_pte`, as the host reaches the program's memory for a
/// pseudo-operation: the hart's permissions apply, but the TLBs are
/// neither consulted nor filled and no A or D bit is set or required.
///
/// # Errors
///
/// The fault the access would take.
pub fn translate_now(
    vaddr: VirtAddr,
    access: AccessType,
    privilege: PrivilegeMode,
    csrs: &Csrs,
    read_pte: impl Fn(PhysAddr) -> Option<u64>,
) -> Result<PhysAddr, Trap> {
    let Some(paging) = Satp::new(csrs.satp).paging_mode() else {
        return Err(Trap::access_fault(access, vaddr.val()));
    };
    if privilege == PrivilegeMode::Machine || paging == PagingMode::Bare {
        return Ok(PhysAddr::new(vaddr.val()));
    }
    if !is_canonical_va(vaddr.val(), paging) {
        return Err(Trap::page_fault(access, vaddr.val()));
    }
    ptw::walk_now(WalkRequest { access, privilege, mode: paging }, vaddr, csrs, read_pte)
}

const fn is_canonical_va(va: u64, mode: PagingMode) -> bool {
    let top = mode.va_top_bit();
    if top >= 63 {
        return true;
    }
    let top_bit = (va >> top) & 1;
    let upper = va >> (top + 1);
    let expected = if top_bit == 1 { (1u64 << (63 - top)) - 1 } else { 0 };
    upper == expected
}

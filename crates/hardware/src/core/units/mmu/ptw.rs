//! Hardware Page Table Walker (PTW) for RISC-V Sv39 / Sv48 / Sv57.
//!
//! The walker is a state machine. The caller (pipeline LSU or front-end)
//! starts a walk via [`start_walk`], stashes the returned [`WalkState`] in
//! its outstanding-walks table, and issues a `MemReq` for the requested
//! `pte_addr`. When the matching `MemResp` arrives, the caller hands the
//! raw PTE back to [`continue_walk`], which either completes translation
//! or returns the next PTE to read.

use crate::common::{
    AccessType, Asid, PAGE_SHIFT, PhysAddr, Ppn, PteUpdate, TranslationResult, Trap, VPN_MASK,
    VirtAddr, Vpn,
};
use crate::core::arch::csr::{
    Csrs, MENVCFG_ADUE, MSTATUS_MXR, MSTATUS_SUM, PagingMode, SATP_ASID_MASK, SATP_ASID_SHIFT,
    SATP_PPN_MASK,
};
use crate::core::arch::mode::PrivilegeMode;
use crate::core::units::mmu::Mmu;
use crate::core::units::mmu::pmp::{Pmp, PmpResult};

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
/// Number of bits used for VPN indexing at each level (9 bits per level).
const VPN_BITS_PER_LEVEL: u64 = 9;
/// Bit mask to extract VPN index from virtual address (9 bits: 0x1FF).
const VPN_ENTRY_MASK: u64 = 0x1FF;
/// Size of a Page Table Entry in bytes.
const PTE_SIZE: u64 = 8;
/// Bits 63-54, reserved for Svnapot, Svpbmt and future extensions.
const PTE_RESERVED_BITS: u64 = u64::MAX << 54;
/// D, A and U, reserved in a pointer PTE.
const PTE_POINTER_RESERVED_BITS: u64 = PTE_DIRTY_BIT | PTE_ACCESSED_BIT | PTE_USER_BIT;
/// Cycles required to update a PTE's accessed/dirty bits in memory.
const PTE_UPDATE_CYCLES: u64 = 10;

/// A strongly-typed wrapper around a raw 64-bit Page Table Entry.
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

    /// True when the PTE sets a bit the spec reserves: bits 63-54 (this
    /// hart implements neither Svnapot nor Svpbmt), or D, A or U in a
    /// pointer to the next level.
    const fn has_reserved_bits(self) -> bool {
        self.0 & PTE_RESERVED_BITS != 0
            || (self.is_pointer() && self.0 & PTE_POINTER_RESERVED_BITS != 0)
    }

    /// Returns a new instance with the Accessed (A) bit set.
    const fn with_accessed(self) -> Self {
        Self(self.0 | PTE_ACCESSED_BIT)
    }
}

/// Per-walk inputs that travel together.
#[derive(Clone, Copy, Debug)]
pub struct WalkRequest {
    /// Type of memory access being attempted (fetch/read/write).
    pub access: AccessType,
    /// Privilege mode of the processor at the time of access.
    pub privilege: PrivilegeMode,
    /// Active paging mode.
    pub mode: PagingMode,
}

/// In-flight page-table walk state.
///
/// Each step issues a [`MemReq`](crate::sim::packet::Packet::MemReq) for the
/// PTE at `pte_addr`. The pipeline stashes the [`WalkState`] until the
/// matching `MemResp` arrives, then calls [`continue_walk`] with the raw
/// 64-bit PTE value to advance.
#[derive(Clone, Debug)]
pub struct WalkState {
    /// Virtual address being translated.
    pub vaddr: VirtAddr,
    /// Type of access.
    pub access: AccessType,
    /// Privilege mode at the start of the walk.
    pub privilege: PrivilegeMode,
    /// Paging mode (drives level count).
    pub mode: PagingMode,
    /// Current page-table level being read (counting down to 0).
    pub level: u32,
    /// PPN of the current page table.
    pub ppn_raw: u64,
    /// ASID captured from SATP at walk start.
    pub asid: Asid,
    /// Cumulative cycle cost of the walk so far.
    pub cycles: u64,
}

/// Output of [`start_walk`] or [`continue_walk`].
#[derive(Clone, Debug)]
pub enum WalkStep {
    /// Walk is finished (success or fault).
    Done(TranslationResult),
    /// Next PTE to read. The caller must issue a `MemReq` for `pte_addr`,
    /// stash the `state`, and call [`continue_walk`] with the resulting
    /// PTE value when the response arrives.
    NeedPte {
        /// Physical address of the next PTE to load.
        pte_addr: PhysAddr,
        /// Walk state to stash until the response arrives.
        state: WalkState,
    },
}

/// Begins a hardware page-table walk for the given virtual address.
///
/// Returns either an immediate [`WalkStep::Done`] (PMP fault before the first
/// PTE read) or [`WalkStep::NeedPte`] with the address of the first PTE.
pub fn start_walk(
    request: WalkRequest,
    vaddr: VirtAddr,
    csrs: &Csrs,
    pmp: Option<&Pmp>,
) -> WalkStep {
    let satp = csrs.satp;
    let ppn_raw = satp & SATP_PPN_MASK;
    let asid = Asid::new(((satp >> SATP_ASID_SHIFT) & SATP_ASID_MASK) as u16);

    let state = WalkState {
        vaddr,
        access: request.access,
        privilege: request.privilege,
        mode: request.mode,
        level: (request.mode.levels() as u32) - 1,
        ppn_raw,
        asid,
        cycles: 0,
    };
    request_pte(state, pmp)
}

/// Continues an in-flight walk after the caller has read the requested PTE
/// from memory.
pub fn continue_walk(
    mut state: WalkState,
    raw_pte: u64,
    mmu: &mut Mmu,
    csrs: &Csrs,
    pmp: Option<&Pmp>,
    bus_transit_cycles: u64,
) -> WalkStep {
    state.cycles += bus_transit_cycles;
    let pte = PageTableEntry::new(raw_pte);

    if !pte.is_valid() {
        return WalkStep::Done(TranslationResult::fault(
            page_fault(state.vaddr.val(), state.access),
            state.cycles,
        ));
    }

    if pte.has_reserved_bits() {
        return WalkStep::Done(TranslationResult::fault(
            page_fault(state.vaddr.val(), state.access),
            state.cycles,
        ));
    }

    if pte.is_pointer() {
        if state.level == 0 {
            return WalkStep::Done(TranslationResult::fault(
                page_fault(state.vaddr.val(), state.access),
                state.cycles,
            ));
        }
        state.level -= 1;
        state.ppn_raw = pte.ppn_raw();
        return request_pte(state, pmp);
    }

    // Leaf PTE: validate, set A/D, install in TLB, return success.
    if pte.can_write() && !pte.can_read() {
        return WalkStep::Done(TranslationResult::fault(
            page_fault(state.vaddr.val(), state.access),
            state.cycles,
        ));
    }

    if state.level > 0 {
        let ppn_mask = (1u64 << (u64::from(state.level) * VPN_BITS_PER_LEVEL)) - 1;
        if (pte.ppn_raw() & ppn_mask) != 0 {
            return WalkStep::Done(TranslationResult::fault(
                page_fault(state.vaddr.val(), state.access),
                state.cycles,
            ));
        }
    }

    if check_permissions(pte, state.access, state.privilege, csrs).is_err() {
        return WalkStep::Done(TranslationResult::fault(
            page_fault(state.vaddr.val(), state.access),
            state.cycles,
        ));
    }

    if csrs.menvcfg & MENVCFG_ADUE == 0 {
        if !pte.is_accessed() {
            return WalkStep::Done(TranslationResult::fault(
                page_fault(state.vaddr.val(), state.access),
                state.cycles,
            ));
        }
        if state.access == AccessType::Write && !pte.is_dirty() {
            return WalkStep::Done(TranslationResult::fault(
                page_fault(state.vaddr.val(), state.access),
                state.cycles,
            ));
        }
    }

    let (accessed_update, dirty_update) = access_bit_updates(&state, pte);
    if accessed_update.is_some() || dirty_update.is_some() {
        state.cycles += PTE_UPDATE_CYCLES;
    }

    let final_ppn = pte.ppn();
    let vpn_shift = PAGE_SHIFT + u64::from(state.level) * VPN_BITS_PER_LEVEL;
    let offset_mask = (1u64 << vpn_shift) - 1;
    let final_paddr = final_ppn.to_addr() | (state.vaddr.val() & offset_mask);

    let specific_4kb_ppn = Ppn::new(final_paddr >> PAGE_SHIFT);
    let vpn = Vpn::new((state.vaddr.val() >> PAGE_SHIFT) & VPN_MASK);

    let pte_raw = pte.raw();
    if state.access == AccessType::Fetch {
        mmu.itlb.insert(vpn, specific_4kb_ppn, pte_raw, state.asid);
    } else {
        mmu.dtlb.insert(vpn, specific_4kb_ppn, pte_raw, state.asid);
    }
    mmu.l2_tlb.insert(vpn, specific_4kb_ppn, pte_raw, state.asid);

    let result = TranslationResult {
        paddr: PhysAddr::new(final_paddr),
        cycles: state.cycles,
        trap: None,
        dirty_update,
        accessed_update,
    };
    WalkStep::Done(result)
}

/// Computes the PTE address for the current walk step, checks PMP, and
/// returns the next [`WalkStep`]. Used by both `start_walk` and
/// `continue_walk` when advancing.
fn request_pte(state: WalkState, pmp: Option<&Pmp>) -> WalkStep {
    let vpn_shift = PAGE_SHIFT + u64::from(state.level) * VPN_BITS_PER_LEVEL;
    let vpn_i = (state.vaddr.val() >> vpn_shift) & VPN_ENTRY_MASK;
    let pte_addr = (state.ppn_raw << PAGE_SHIFT) + (vpn_i * PTE_SIZE);

    if let Some(pmp_unit) = pmp {
        let pmp_result = pmp_unit.check(pte_addr, 8, true, false, false, false);
        if pmp_result != PmpResult::Allow {
            let trap = match state.access {
                AccessType::Fetch => Trap::InstructionAccessFault(state.vaddr.val()),
                AccessType::Read => Trap::LoadAccessFault(state.vaddr.val()),
                AccessType::Write => Trap::StoreAccessFault(state.vaddr.val()),
            };
            return WalkStep::Done(TranslationResult::fault(trap, state.cycles));
        }
    }

    WalkStep::NeedPte { pte_addr: PhysAddr::new(pte_addr), state }
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
    if access == AccessType::Write && !pte.can_write() {
        return Err(());
    }
    if access == AccessType::Fetch && !pte.can_exec() {
        return Err(());
    }

    let mxr = csrs.mstatus & MSTATUS_MXR != 0;

    if access == AccessType::Read && !(pte.can_read() || (pte.can_exec() && mxr)) {
        return Err(());
    }

    if privilege == PrivilegeMode::User && !pte.is_user() {
        return Err(());
    }

    if privilege == PrivilegeMode::Supervisor && pte.is_user() {
        let sum = csrs.mstatus & MSTATUS_SUM != 0;
        if !sum {
            return Err(());
        }
        if access == AccessType::Fetch {
            return Err(());
        }
    }

    Ok(())
}

/// The hardware A/D updates the access that walked to leaf `pte` needs:
/// setting A, which the walker does at once, and setting D for a store,
/// which waits until the store retires.
fn access_bit_updates(
    state: &WalkState,
    pte: PageTableEntry,
) -> (Option<PteUpdate>, Option<PteUpdate>) {
    let vpn_shift = PAGE_SHIFT + u64::from(state.level) * VPN_BITS_PER_LEVEL;
    let vpn_i = (state.vaddr.val() >> vpn_shift) & VPN_ENTRY_MASK;
    let pte_addr = PhysAddr::new((state.ppn_raw << PAGE_SHIFT) + (vpn_i * PTE_SIZE));
    let accessed = (!pte.is_accessed()).then_some(PteUpdate {
        pte_addr,
        walked_pte: pte.raw(),
        set_bits: PTE_ACCESSED_BIT,
    });
    let dirty = (state.access == AccessType::Write && !pte.is_dirty()).then_some(PteUpdate {
        pte_addr,
        walked_pte: pte.with_accessed().raw(),
        set_bits: PTE_ACCESSED_BIT | PTE_DIRTY_BIT,
    });
    (accessed, dirty)
}

/// Constructs the appropriate Trap for a failed page access.
const fn page_fault(addr: u64, access: AccessType) -> Trap {
    match access {
        AccessType::Fetch => Trap::InstructionPageFault(addr),
        AccessType::Read => Trap::LoadPageFault(addr),
        AccessType::Write => Trap::StorePageFault(addr),
    }
}

//! Virtual-to-physical translation entry point on `SystemState`.
//!
//! Wraps the MMU's event-driven [`Mmu::translate_async`](crate::uarch::mmu::Mmu::translate_async)
//! and PMP checks. Pipeline stages call this; on a TLB hit / direct-mode
//! address the result is immediate, on a TLB miss the caller stashes the
//! returned walk state until the PTE response arrives in its mailbox.

use super::CoreCtx;
use crate::arch::Hart;
use crate::arch::pmp::PmpResult;
use crate::arch::translation::TranslationResult;
use crate::common::{AccessType, PAGE_SHIFT, PhysAddr, VirtAddr};
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::sim::memory::write_log::Writer;
use crate::soc::uncore::Uncore;
use crate::uarch::CoreUnits;
use crate::uarch::mmu::ptw::WalkState;
use crate::uarch::mmu::{self, TranslateOutcome};

/// How many bytes lie between `vaddr` and the end of its page.
const fn bytes_to_page_end(vaddr: u64) -> usize {
    const PAGE_SIZE: u64 = 1 << PAGE_SHIFT;
    (PAGE_SIZE - (vaddr & (PAGE_SIZE - 1))) as usize
}

/// Begins (or completes) translation of a virtual address.
pub(super) fn translate(
    core: &mut CoreUnits,
    hart: &Hart,
    uncore: &Uncore,
    vaddr: VirtAddr,
    access: AccessType,
    size: u64,
) -> TranslateOutcome {
    let effective_priv =
        if access == AccessType::Fetch { hart.privilege } else { data_privilege(hart) };

    let outcome =
        core.mmu.translate_async(vaddr, access, effective_priv, &hart.csrs, Some(&hart.pmp));

    finalize_outcome(hart, uncore, outcome, vaddr, access, size, effective_priv)
}

/// The privilege a load or store translates and is checked at: the mode
/// in `mstatus.MPP` while `mstatus.MPRV` is set, the hart's own otherwise.
pub(super) const fn data_privilege(hart: &Hart) -> PrivilegeMode {
    use crate::isa::csr::{MSTATUS_MPP_MASK, MSTATUS_MPP_SHIFT, MSTATUS_MPRV};

    if hart.csrs.mstatus & MSTATUS_MPRV == 0 {
        return hart.privilege;
    }
    let mpp = ((hart.csrs.mstatus >> MSTATUS_MPP_SHIFT) & MSTATUS_MPP_MASK) as u8;
    PrivilegeMode::from_u8(mpp)
}

/// Resumes a walk that was parked waiting on a PTE response.
pub(super) fn translate_continue(
    core: &mut CoreUnits,
    hart: &Hart,
    uncore: &Uncore,
    state: WalkState,
    raw_pte: u64,
    bus_transit_cycles: u64,
) -> TranslateOutcome {
    let vaddr = state.vaddr;
    let access = state.access;
    let effective_priv = state.privilege;
    // The walk state carries its own size context only for fault reporting;
    // PMP needs the access size. Translation post-checks size against PMP
    // again once the leaf PTE resolves, but the walk itself reads 8 bytes
    // per PTE which is what `start_walk` / `continue_walk` enforce.
    let size = 8u64;
    let outcome =
        core.mmu.continue_walk(state, raw_pte, &hart.csrs, Some(&hart.pmp), bus_transit_cycles);
    finalize_outcome(hart, uncore, outcome, vaddr, access, size, effective_priv)
}

/// Applies the post-translation PMP + bus-address checks shared by the
/// initial translate and walk continuation paths.
fn finalize_outcome(
    hart: &Hart,
    uncore: &Uncore,
    outcome: TranslateOutcome,
    vaddr: VirtAddr,
    access: AccessType,
    size: u64,
    effective_priv: crate::isa::privileged::PrivilegeMode,
) -> TranslateOutcome {
    match outcome {
        TranslateOutcome::Ready(mut result) => {
            if result.trap.is_none() {
                let paddr = result.paddr.val();
                let is_machine = effective_priv == crate::isa::privileged::PrivilegeMode::Machine;
                let pmp_result = hart.pmp.check(
                    paddr,
                    size,
                    matches!(access, AccessType::Read),
                    matches!(access, AccessType::Write),
                    matches!(access, AccessType::Fetch),
                    is_machine,
                );
                if pmp_result != PmpResult::Allow || !uncore.bus.is_valid_address(result.paddr) {
                    result = TranslationResult::fault(
                        Trap::access_fault(access, vaddr.val()),
                        result.cycles,
                    );
                }
            }
            TranslateOutcome::Ready(result)
        }
        need_pte @ TranslateOutcome::NeedPte { .. } => need_pte,
    }
}

impl CoreCtx<'_> {
    /// Begins (or completes) translation of a virtual address.
    pub fn translate(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        size: u64,
    ) -> TranslateOutcome {
        translate(self.core, self.hart, self.uncore, vaddr, access, size)
    }

    /// The physical address a data `access` to `vaddr` reaches at the
    /// hart's current privilege, walked now: how the host reaches the
    /// program's memory for a pseudo-operation, with the hart's
    /// permissions but without touching the TLBs or the A and D bits.
    ///
    /// # Errors
    ///
    /// The fault the access would take.
    pub fn translate_now(&self, vaddr: VirtAddr, access: AccessType) -> Result<PhysAddr, Trap> {
        let memory = &self.uncore.memory;
        mmu::translate_now(vaddr, access, data_privilege(self.hart), &self.hart.csrs, |pte_addr| {
            memory.read(pte_addr, 8)
        })
    }

    /// Reads `len` bytes of the program's memory at `vaddr`, translated
    /// now page by page.
    ///
    /// # Errors
    ///
    /// The fault a load of the first byte that cannot be read would take.
    pub fn read_now(&self, vaddr: u64, len: usize) -> Result<Vec<u8>, Trap> {
        let mut out = Vec::new();
        let mut cursor = vaddr;
        let mut left = len;
        while left > 0 {
            let paddr = self.translate_now(VirtAddr::new(cursor), AccessType::Read)?;
            let chunk = left.min(bytes_to_page_end(cursor));
            let bytes =
                self.uncore.memory.read_bytes(paddr, chunk).ok_or(Trap::LoadAccessFault(cursor))?;
            out.extend_from_slice(&bytes);
            cursor = cursor.wrapping_add(chunk as u64);
            left -= chunk;
        }
        Ok(out)
    }

    /// Reads the NUL-terminated string at `vaddr`, up to `limit` bytes,
    /// translated now.
    ///
    /// # Errors
    ///
    /// The fault a load of the first byte that cannot be read would take.
    pub fn read_cstr_now(&self, vaddr: u64, limit: usize) -> Result<Vec<u8>, Trap> {
        let mut out = Vec::new();
        let mut cursor = vaddr;
        while out.len() < limit {
            let chunk = (limit - out.len()).min(bytes_to_page_end(cursor));
            let bytes = self.read_now(cursor, chunk)?;
            match bytes.iter().position(|&byte| byte == 0) {
                Some(end) => {
                    out.extend_from_slice(&bytes[..end]);
                    return Ok(out);
                }
                None => out.extend_from_slice(&bytes),
            }
            cursor = cursor.wrapping_add(chunk as u64);
        }
        Ok(out)
    }

    /// Writes `bytes` to the program's memory at `vaddr`, translated now
    /// page by page, as an external write every hart observes.
    ///
    /// # Errors
    ///
    /// The fault a store to the first byte that cannot be written would
    /// take; the bytes before it are written.
    pub fn write_now(&mut self, vaddr: u64, bytes: &[u8]) -> Result<(), Trap> {
        let mut cursor = vaddr;
        let mut rest = bytes;
        while !rest.is_empty() {
            let paddr = self.translate_now(VirtAddr::new(cursor), AccessType::Write)?;
            let (chunk, tail) = rest.split_at(rest.len().min(bytes_to_page_end(cursor)));
            if !self.uncore.bus.is_ram(paddr, chunk.len() as u64) {
                return Err(Trap::StoreAccessFault(cursor));
            }
            self.uncore.memory.write_bytes(Writer::External, paddr, chunk);
            cursor = cursor.wrapping_add(chunk.len() as u64);
            rest = tail;
        }
        Ok(())
    }

    /// Resumes a walk that was parked waiting on a PTE response.
    pub fn translate_continue(
        &mut self,
        state: WalkState,
        raw_pte: u64,
        bus_transit_cycles: u64,
    ) -> TranslateOutcome {
        translate_continue(self.core, self.hart, self.uncore, state, raw_pte, bus_transit_cycles)
    }
}

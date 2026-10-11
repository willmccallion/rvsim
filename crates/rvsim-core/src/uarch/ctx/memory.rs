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
use crate::common::{AccessType, VirtAddr};
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::soc::uncore::Uncore;
use crate::uarch::CoreUnits;
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::mmu::ptw::WalkState;

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

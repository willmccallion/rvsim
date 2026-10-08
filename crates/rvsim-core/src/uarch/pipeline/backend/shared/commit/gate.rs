//! What stops or pre-empts retirement: a parked trap, a pending interrupt,
//! a waiting WFI, and the conditions under which the ROB head may not
//! retire this cycle.

use crate::arch::reservation::LrScRecord;
use crate::arch::translation::PteUpdate;
use crate::common::PhysAddr;
use crate::exec::retire;
use crate::isa::fence::Fence;
use crate::isa::op::{MemWidth, SystemOp};
use crate::isa::privileged::Trap;
use crate::trace_trap;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::engine::{BackendCommon, PendingTrap, TrapProgress};
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::lsq::write_buffer::WriteCombiningBuffer;
use crate::uarch::pipeline::rob::{Rob, RobEntry, RobState};

use super::{CommitEvent, CommitFlow, CommitRegisters, ReExecuteCause};

/// Takes a parked trap once its latency has elapsed, or an interrupt once
/// the pipeline has drained for it, and holds commit while a WFI waits.
pub(super) fn trap_or_interrupt(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    rob: &Rob,
) -> CommitFlow {
    if let TrapProgress::Pending(pending) = &common.trap {
        state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
        if state.cycle < pending.taken_at {
            return CommitFlow::Stop(None);
        }
        let pending = pending.clone();
        common.trap = TrapProgress::None;
        return CommitFlow::Stop(take_pending_trap(state, pending));
    }

    // Checked even with an empty ROB: a timer can fire during a stall.
    let epc = rob.peek_head().map_or(state.hart.pc, |head| head.pc);
    let interrupt =
        retire::pending_interrupt(state.hart).filter(|_| !device_access_in_flight(common, rob));
    if let Some(interrupt_trap) = interrupt {
        // Fetch stops and everything already fetched retires first
        // (gem5 waits for its instruction list to empty); a WFI's
        // successors are wrong-path and are not waited for.
        common.trap = TrapProgress::DrainingForInterrupt;
        let drained = state.hart.wfi_waiting || (rob.is_empty() && common.frontend_empty);
        if drained {
            trace_trap!(state.trace_trap_enabled(&interrupt_trap);
                event      = "interrupt",
                epc        = %crate::common::trace::Hex(epc),
                cause      = ?interrupt_trap,
                mip        = %crate::common::trace::Hex(state.hart.csrs.mip),
                mie        = %crate::common::trace::Hex(state.hart.csrs.mie),
                mstatus    = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: interrupt detected — pipeline drained"
            );
            state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
            return CommitFlow::Stop(schedule_trap(state, common, interrupt_trap, epc));
        }
        return CommitFlow::Continue;
    }

    common.trap = TrapProgress::None;
    if !state.hart.wfi_waiting {
        return CommitFlow::Continue;
    }
    // Commit stops while the WFI waits; what was fetched behind it is
    // refetched when it wakes.
    state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
    if state.hart.csrs.mip & state.hart.csrs.mie != 0 {
        state.hart.wfi_waiting = false;
        return CommitFlow::Stop(Some(CommitEvent::SquashAfter(state.hart.pc)));
    }
    state.uncore.stats.counter(state.core.stat_paths.pipeline.cycles_wfi).inc();
    CommitFlow::Stop(None)
}

/// Decides whether the ROB head retires this cycle. A faulted head is
/// popped here and its trap scheduled.
pub(super) fn gate_head(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    rob: &mut Rob,
    store_buffer: &StoreBuffer,
    vec_store_buffer: &VecStoreBuffer,
    registers: &mut CommitRegisters<'_>,
) -> CommitFlow {
    let Some(head) = rob.peek_head() else { return CommitFlow::Stop(None) };

    // A squash on its way will remove the head: it is wrong-path.
    if common.will_squash(head.tag) {
        return CommitFlow::Stop(None);
    }

    // Block load retirement while older stores have unresolved addresses,
    // so memory2 can still flag a violation against a later-resolving store.
    if head.state == RobState::Completed
        && head.ctrl.mem_read
        && store_buffer.has_unresolved_store_before(head.tag)
    {
        return CommitFlow::Stop(None);
    }

    if head.state == RobState::Issued {
        return CommitFlow::Stop(None);
    }

    // A store whose address half has completed retires once its data half
    // has delivered the data.
    if head.state == RobState::Completed
        && head.ctrl.splits_store()
        && !store_buffer.has_data(head.tag)
    {
        return CommitFlow::Stop(None);
    }

    if head.state == RobState::Faulted {
        return CommitFlow::Stop(take_fault(state, common, rob, registers));
    }

    if head.state == RobState::Completed && lr_read_a_stale_line(state, head) {
        state.uncore.stats.counter(state.core.stat_paths.lsq.coherence_replays).inc();
        trace_trap!(state.config.general.trace_instructions;
            event   = "coherence-reexecute",
            pc      = %crate::common::trace::Hex(head.pc),
            rob_tag = %head.tag,
            "CM: LR read a line another hart has since written — re-executing"
        );
        return CommitFlow::Stop(Some(CommitEvent::ReExecute(head.pc, ReExecuteCause::StaleLine)));
    }

    // Setting D must recheck the PTE the store was translated with.
    if head.state == RobState::Completed
        && head.dirty_updates.iter().any(|update| updated_pte(state, update).is_none())
    {
        trace_trap!(state.config.general.trace_instructions;
            event   = "pte-changed-reexecute",
            pc      = %crate::common::trace::Hex(head.pc),
            rob_tag = %head.tag,
            "CM: store's PTE changed since its walk — re-executing"
        );
        return CommitFlow::Stop(Some(CommitEvent::ReExecute(head.pc, ReExecuteCause::ChangedPte)));
    }

    // A barrier retires once every older store's write has completed.
    if waits_for_older_stores(head)
        && older_stores_pending(store_buffer, vec_store_buffer, &state.core.wcb)
    {
        return CommitFlow::Stop(None);
    }

    CommitFlow::Continue
}

/// Pops the faulted ROB head, commits the vector elements it produced
/// before its fault, frees its destinations and schedules its trap.
fn take_fault(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    rob: &mut Rob,
    registers: &mut CommitRegisters<'_>,
) -> Option<CommitEvent> {
    let entry = rob.commit_head()?;
    let the_trap = entry.trap.as_ref()?;
    #[cfg(feature = "commit-log")]
    if let Some(ref mut log) = state.commit_log {
        use std::io::Write;
        // Spike skips fetch-stage page/access faults (no valid bits).
        let skip = matches!(
            the_trap,
            Trap::InstructionPageFault(_)
                | Trap::InstructionAccessFault(_)
                | Trap::InstructionAddressMisaligned(_)
        );
        if !skip {
            let _ = writeln!(log, "core   0: 0x{:016x} (0x{:08x})", entry.pc, entry.inst);
        }
    }
    trace_trap!(state.trace_trap_enabled(the_trap);
        event     = "sync-exception",
        pc        = %crate::common::trace::Hex(entry.pc),
        rob_tag   = %entry.tag,
        cause     = ?the_trap,
        priv_mode = ?state.hart.privilege,
        mstatus   = %crate::common::trace::Hex(state.hart.csrs.mstatus),
        "CM: synchronous exception at commit"
    );
    // Elements a vector load returned before the faulting one.
    if let Some(writes) = &entry.vec_writes {
        writes.apply(state.hart.regs.vpr_mut());
    }
    if let Some(vstart) = entry.fault_vstart {
        state.hart.csrs.vstart = vstart;
    }
    // Faulting entry was popped before the post-trap flush, so reclaim its phys_dst here.
    registers.reclaim_faulted(&entry);
    schedule_trap(state, common, the_trap.clone(), entry.pc)
}

/// True when the LR at the ROB head read its line before another hart
/// wrote it, so the reservation it would set is already broken. Plain loads
/// are not checked: RVWMO lets them keep the earlier value.
fn lr_read_a_stale_line(state: &CoreCtx<'_>, head: &RobEntry) -> bool {
    let Some(log) = state.memory.write_log() else { return false };
    let (Some(observed), Some(LrScRecord::Lr { paddr })) = (head.observed, head.lr_sc) else {
        return false;
    };
    log.written_by_other_since(paddr, state.hart.hart_id, observed)
}

/// The PTE a hardware A/D update writes, or `None` when the PTE has changed
/// since the walk that produced it.
pub(super) fn updated_pte(state: &CoreCtx<'_>, update: &PteUpdate) -> Option<u64> {
    read_ram_word(state, update.pte_addr, MemWidth::Double)
        .and_then(|current| update.applied_to(current))
}

/// The word at `paddr` as it is in RAM right now; `None` outside pure RAM.
fn read_ram_word(state: &CoreCtx<'_>, paddr: PhysAddr, width: MemWidth) -> Option<u64> {
    if width == MemWidth::Nop || !state.bus.is_ram(paddr, width.bytes()) {
        return None;
    }
    state.memory.read(paddr, width.bytes() as usize)
}

/// Takes `trap` now when there is no trap latency, else parks it for
/// commit to take once the latency has elapsed.
fn schedule_trap(
    state: &CoreCtx<'_>,
    common: &mut BackendCommon,
    trap: Trap,
    epc: u64,
) -> Option<CommitEvent> {
    let latency = state.config.pipeline.trap_latency;
    if latency == 0 {
        return Some(CommitEvent::Trap(trap, epc));
    }
    common.trap = TrapProgress::Pending(PendingTrap { trap, epc, taken_at: state.cycle + latency });
    None
}

/// Takes a parked trap. An interrupt is what is pending and enabled now,
/// which may differ from what was detected, or be nothing at all.
fn take_pending_trap(state: &mut CoreCtx<'_>, pending: PendingTrap) -> Option<CommitEvent> {
    let (is_interrupt, _) = pending.trap.cause();
    let trap = if is_interrupt { retire::pending_interrupt(state.hart)? } else { pending.trap };
    if is_interrupt {
        state.hart.wfi_waiting = false;
    }
    Some(CommitEvent::Trap(trap, pending.epc))
}

/// True when the ROB head has a device read outstanding. Such a read was
/// issued non-speculatively and has already had its side effect, so the
/// instruction must retire before an interrupt can pre-empt it.
fn device_access_in_flight(common: &BackendCommon, rob: &Rob) -> bool {
    rob.peek_head().is_some_and(|head| {
        common.outstanding_loads.values().any(|l| l.side_effecting && l.entry.rob_tag == head.tag)
    })
}

/// True when `head` must not retire before every older store's write has
/// completed: SFENCE.VMA (the walker must see earlier PTE stores), FENCE.I,
/// and a FENCE whose predecessor set includes writes. An atomic that needs
/// older stores written waits for them before it issues, and a CBO is
/// ordered with them in the store buffer.
fn waits_for_older_stores(head: &RobEntry) -> bool {
    matches!(head.ctrl.system_op, SystemOp::SfenceVma | SystemOp::FenceI)
        || (head.ctrl.system_op == SystemOp::Fence && Fence::decode(head.inst).pred.w)
}

/// True while a committed scalar or vector store has not finished writing.
pub(super) fn older_stores_pending(
    store_buffer: &StoreBuffer,
    vec_store_buffer: &VecStoreBuffer,
    wcb: &WriteCombiningBuffer,
) -> bool {
    store_buffer.has_committed_stores()
        || vec_store_buffer.has_committed_stores()
        || wcb.has_pending()
}

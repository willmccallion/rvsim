//! Memory1 stage: translation, alignment, and `MemReq` issuance.
//!
//! For each `ExMem1Entry` coming in from execute, memory1:
//!
//! - **Trap propagation** — the entry already carries a trap: pass it
//!   straight through to the M1→M2 latch so writeback can mark the ROB
//!   entry faulted at its normal stage.
//! - **Non-memory ops** — pass through to M1→M2; memory2 / writeback handle
//!   the ALU pass-through.
//! - **Memory ops** — perform alignment + load/store trigger checks, then
//!   translate the virtual address.
//!   - On a [`TranslateResult::NeedPte`], park the entry under an
//!     [`OutstandingWalk`] with [`WalkContinuation::LoadStore`] and emit a
//!     `MemReq` for the PTE. When the walk completes the mailbox drain
//!     hands the entry back here with the walk's translation.
//!   - On a fault (PMP / page fault / unmapped paddr), emit a trapped
//!     `Mem1Mem2Entry`.
//!   - For demand **loads**: check store-buffer forwarding first.
//!     - SB hit → push directly to M1→M2 with `load_data` filled and
//!       `sb_forwarded = true`. No `MemReq` issued.
//!     - SB partial overlap → move to `BackendCommon::mem1_replay` and retry
//!       next cycle; the store it overlaps must drain first.
//!     - SB miss → emit `MemReq` to L1D and park [`OutstandingLoad`].
//!   - For **AMO / LR**: replay while any older store to the same address is
//!     still resident in the SB. Otherwise emit `MemReq` and park.
//!   - For **stores**: pass to M1→M2 with the resolved `paddr`. Memory2
//!     resolves the store buffer and checks for ordering violations.
//!   - For **SC**: same as stores, plus an `AtomicOp::Sc` marker so memory2
//!     records the deferred `LrScRecord::Sc`.

use crate::common::TranslationResult;
use crate::common::{AccessType, DirtyUpdates, ExceptionStage, PhysAddr, Trap, VirtAddr};
use crate::core::pipeline::backend::shared::cbo;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry};
use crate::core::pipeline::outstanding::{
    DelayedAccess, ForwardedLoad, OutstandingLoad, OutstandingWalk, PageTranslations,
    WalkContinuation,
};
use crate::core::pipeline::rob::{RobState, RobTag};
use crate::core::pipeline::signals::{AtomicOp, MemWidth};
use crate::core::pipeline::store_buffer::ForwardResult;
use crate::core::units::lsu::unaligned;
use crate::core::units::vpu::types::ElemIdx;
use crate::isa::zicboz::CBOZ_BLOCK_SIZE;
use crate::sim::StageCtx;
use crate::sim::components::ComponentId;
use crate::sim::packet::{self, AccessSize, MemOp, Packet};
use crate::sim::state::memory::TranslateResult;
use crate::{trace_fwd, trace_mem};

/// Outcome of processing a single `ExMem1Entry`.
enum EntryOutcome {
    /// Entry was passed downstream (`Mem1Mem2` push); continue iterating.
    Done,
    /// SB partial overlap or atomic-vs-SB conflict — retry on a later tick
    /// once the blocking store has drained. Only this op waits; younger
    /// memory ops keep flowing.
    Replay(ExMem1Entry),
    /// Entry was parked on a page-table walk in the engine's outstanding
    /// tables. Younger memory ops cannot pass an unresolved older
    /// translation in an in-order pipeline, so halt iteration and push any
    /// remaining entries back to the input latch.
    ParkedWalk,
    /// The translation hit the L2 TLB; the op continues after its latency.
    Delayed(DelayedAccess),
}

/// What memory1 resolved this cycle, for the engine to act on.
#[derive(Debug, Default)]
pub struct Memory1Outcome {
    /// Stores whose address and data entered the store buffer, in order.
    pub resolved_stores: Vec<RobTag>,
    /// The oldest younger load a resolving store found had already read
    /// the location, and that store's PC.
    pub violation: Option<(RobTag, u64)>,
}

/// Executes the Memory1 stage.
///
/// A plain store resolves its store-buffer slot here, once translated, and
/// checks the load queue for younger loads that already read the location;
/// store-conditionals, AMOs and vector stores resolve in memory2, where their
/// data is final.
pub fn memory1_stage<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    input: &mut Vec<ExMem1Entry>,
) -> Memory1Outcome {
    let mut outcome = Memory1Outcome::default();
    let now = state.cycle;
    release_forwarded_loads(engine, now);
    let mut entries = std::mem::take(&mut engine.common_mut().mem1_replay);
    entries.append(input);
    let delayed = &mut engine.common_mut().mem1_delayed;
    let mut ready: Vec<DelayedAccess> = Vec::new();
    delayed.retain(|access| {
        if access.ready_cycle <= now {
            ready.push(access.clone());
            false
        } else {
            true
        }
    });
    let mut translations: Vec<(RobTag, Option<ElemIdx>, PageTranslations)> = ready
        .iter()
        .map(|a| {
            let elem = a.entry.vec_mem.as_ref().map(|v| v.elem_idx);
            (a.entry.rob_tag, elem, a.translations.clone())
        })
        .collect();
    entries.extend(ready.into_iter().map(|a| a.entry));
    // Out-of-order execute can drop entries into execute_mem1 in completion
    // order rather than program order. memory1's SB-forward / atomic-vs-SB
    // checks only inspect *older* store entries, so process oldest first to
    // give each op the most-drained store buffer view available this cycle.
    entries.sort_by_key(|e| e.rob_tag.0);
    let mut iter = entries.into_iter();

    while let Some(ex) = iter.next() {
        let elem = ex.vec_mem.as_ref().map(|v| v.elem_idx);
        let translated = translations
            .iter()
            .position(|(tag, e, _)| *tag == ex.rob_tag && *e == elem)
            .map(|i| translations.swap_remove(i).2)
            .unwrap_or_default();
        match process_entry(state, engine, ex, translated, &mut outcome) {
            EntryOutcome::Done => {}
            EntryOutcome::Replay(ex) => {
                state.counter(state.core().stat_paths.lsq.rescheduled_mem_ops).inc();
                engine.common_mut().mem1_replay.push(ex);
            }
            EntryOutcome::Delayed(access) => engine.common_mut().mem1_delayed.push(access),
            EntryOutcome::ParkedWalk => {
                input.extend(iter);
                return outcome;
            }
        }
    }
    outcome
}

/// Moves forwarded loads whose L1D latency has elapsed into the M1→M2 latch.
fn release_forwarded_loads<E: ExecutionEngine>(engine: &mut E, now: u64) {
    let mut ready = Vec::new();
    engine.common_mut().forwarded_loads.retain(|load| {
        if load.ready_cycle <= now {
            ready.push(load.entry.clone());
            false
        } else {
            true
        }
    });
    engine.mem1_mem2_mut().extend(ready);
}

/// Processes one entry; `translated` holds the translations an access that
/// waited out an L2 TLB hit or a page-table walk already has.
fn process_entry<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    translated: PageTranslations,
    resolved: &mut Memory1Outcome,
) -> EntryOutcome {
    // 1. Trap propagation. An entry execute already faulted carries its
    // trap from here on and performs no access.
    if ex.trap.is_some() {
        push_passthrough_with_trap(engine, ex);
        return EntryOutcome::Done;
    }
    if let Some(faulted) =
        engine.rob().find_entry(ex.rob_tag).filter(|e| e.state == RobState::Faulted)
    {
        let mut ex = ex;
        ex.trap.clone_from(&faulted.trap);
        ex.exception_stage = faulted.exception_stage;
        push_passthrough_with_trap(engine, ex);
        return EntryOutcome::Done;
    }

    if ex.ctrl.system_op.is_cbo() {
        return translate_cbo(state, engine, ex, translated.first);
    }

    let needs_translation = ex.ctrl.mem_read || ex.ctrl.mem_write;
    if !needs_translation {
        push_passthrough(engine, ex);
        return EntryOutcome::Done;
    }

    // 2. Alignment.
    let size = unaligned::width_to_bytes(ex.ctrl.width);
    let is_atomic = ex.ctrl.atomic_op != AtomicOp::None;
    if !unaligned::is_aligned(ex.alu, size)
        && (state.config.memory.misaligned_access_trap || is_atomic)
    {
        let trap = if ex.ctrl.mem_write {
            unaligned::store_misaligned_trap(ex.alu)
        } else {
            unaligned::load_misaligned_trap(ex.alu)
        };
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    // 3. Sdtrig load/store triggers.
    if ex.ctrl.mem_read && !is_atomic && state.check_load_trigger(ex.alu) {
        let trap = Trap::Breakpoint(ex.pc);
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }
    if ex.ctrl.mem_write && !is_atomic && state.check_store_trigger(ex.alu) {
        let trap = Trap::Breakpoint(ex.pc);
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    // 4. Translation. An L2 TLB hit refills the L1 and costs its latency
    // before the access continues; the L1 TLB answers in the same cycle.
    let access_type = if ex.ctrl.mem_write { AccessType::Write } else { AccessType::Read };
    let outcome = translated.first.map_or_else(
        || state.translate(VirtAddr::new(ex.alu), access_type, size),
        TranslateResult::Ready,
    );
    let first = match outcome {
        TranslateResult::Ready(r) => {
            if let Some(trap) = r.trap {
                push_trap(engine, ex, trap, ExceptionStage::Memory);
                return EntryOutcome::Done;
            }
            if r.cycles > 0 {
                let first = Some(TranslationResult { cycles: 0, ..r });
                let translations = PageTranslations { first, second: None };
                return EntryOutcome::Delayed(DelayedAccess {
                    ready_cycle: state.cycle + r.cycles,
                    entry: ex,
                    translations,
                });
            }
            r
        }
        TranslateResult::NeedPte { pte_addr, state: walk_state } => {
            park_walk(state, engine, walk_state, pte_addr, ex, PageTranslations::default());
            return EntryOutcome::ParkedWalk;
        }
    };
    let paddr = first.paddr;

    // 4b. A misaligned access that spills into the next page translates
    // that page as well; the two halves must then be physically adjacent
    // for the single-request data path, otherwise the access is left to
    // the misaligned trap handler like a real split-unaware LSU.
    let mut second_dirty_update = None;
    if let Some(second_va) = unaligned::second_page_start(ex.alu, size) {
        let outcome = translated.second.map_or_else(
            || state.translate(VirtAddr::new(second_va), access_type, 1),
            TranslateResult::Ready,
        );
        match outcome {
            TranslateResult::Ready(r) => {
                if let Some(trap) = r.trap {
                    push_trap(engine, ex, trap, ExceptionStage::Memory);
                    return EntryOutcome::Done;
                }
                if r.cycles > 0 {
                    let second = Some(TranslationResult { cycles: 0, ..r });
                    let translations = PageTranslations { first: Some(first), second };
                    return EntryOutcome::Delayed(DelayedAccess {
                        ready_cycle: state.cycle + r.cycles,
                        entry: ex,
                        translations,
                    });
                }
                let first_page_bytes = second_va.wrapping_sub(ex.alu);
                if r.paddr.val() != paddr.val().wrapping_add(first_page_bytes) {
                    let trap = if ex.ctrl.mem_write {
                        unaligned::store_misaligned_trap(ex.alu)
                    } else {
                        unaligned::load_misaligned_trap(ex.alu)
                    };
                    push_trap(engine, ex, trap, ExceptionStage::Memory);
                    return EntryOutcome::Done;
                }
                second_dirty_update = r.dirty_update;
            }
            TranslateResult::NeedPte { pte_addr, state: walk_state } => {
                let translations = PageTranslations { first: Some(first), second: None };
                park_walk(state, engine, walk_state, pte_addr, ex, translations);
                return EntryOutcome::ParkedWalk;
            }
        }
    }
    let dirty_updates = DirtyUpdates::of(first.dirty_update, second_dirty_update);

    // 5. Load-queue address fill (O3).
    if ex.ctrl.mem_read
        && let Some(lq) = engine.load_queue_mut()
    {
        let elem = ex.vec_mem.as_ref().map(|v| v.elem_idx);
        lq.fill_address(ex.rob_tag, elem, VirtAddr::new(ex.alu), paddr);
    }

    // 6. Operation dispatch.
    let vaddr = VirtAddr::new(ex.alu);

    if ex.ctrl.mem_write && !is_atomic {
        if ex.vec_mem.is_none() {
            resolve_store(state, engine, &ex, paddr, vaddr, resolved);
        }
        push_resolved_store(engine, ex, paddr, vaddr, dirty_updates);
        return EntryOutcome::Done;
    }

    if is_atomic {
        if ex.ctrl.atomic_op == AtomicOp::Sc {
            push_resolved_store(engine, ex, paddr, vaddr, dirty_updates);
            return EntryOutcome::Done;
        }
        // LR / AMO: wait for older stores to this address to drain.
        if engine.store_buffer().has_older_store_to(paddr, ex.ctrl.width, ex.rob_tag) {
            return EntryOutcome::Replay(ex);
        }
        if reads_a_device(state, paddr, size) && !is_rob_head(engine, ex.rob_tag) {
            return EntryOutcome::Replay(ex);
        }
        emit_load_req(state, engine, ex, paddr, vaddr, dirty_updates, true);
        return EntryOutcome::Done;
    }

    // A device read has side effects, so it waits until nothing older can
    // still fault, redirect or be interrupted: the load must be the oldest
    // instruction in the machine.
    if reads_a_device(state, paddr, size) && !is_rob_head(engine, ex.rob_tag) {
        return EntryOutcome::Replay(ex);
    }

    // Demand load: try store-buffer forwarding first, from scalar stores
    // and then from vector stores still in their buffer.
    let forwarded = match engine.store_buffer().forward_load(paddr, ex.ctrl.width, ex.rob_tag) {
        ForwardResult::Miss => {
            engine.vec_store_buffer().forward_load(paddr, ex.ctrl.width, ex.rob_tag)
        }
        scalar => scalar,
    };
    match forwarded {
        ForwardResult::Hit(raw_val) => {
            push_sb_forwarded_load(state, engine, ex, paddr, vaddr, dirty_updates, raw_val);
            EntryOutcome::Done
        }
        ForwardResult::Stall => EntryOutcome::Replay(ex),
        ForwardResult::Miss => {
            emit_load_req(state, engine, ex, paddr, vaddr, dirty_updates, false);
            EntryOutcome::Done
        }
    }
}

/// Translates a cache-block operation's block and passes its physical
/// address on as the entry's result for commit, which performs it there.
/// A fault is reported as the store fault the CBO raises.
fn translate_cbo<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    mut ex: ExMem1Entry,
    translated: Option<TranslationResult>,
) -> EntryOutcome {
    let hart = state.hart();
    let effect = match cbo::gate(&hart.csrs, hart.privilege, ex.ctrl.system_op, ex.inst) {
        Ok(effect) => effect,
        Err(trap) => {
            push_trap(engine, ex, trap, ExceptionStage::Memory);
            return EntryOutcome::Done;
        }
    };
    let rs1 = ex.alu;
    let block = cbo::block_address(rs1);
    let tval = cbo::fault_address(ex.ctrl.system_op, rs1);
    if state.check_store_trigger(block) {
        let trap = Trap::Breakpoint(ex.pc);
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    let outcome = translated.map_or_else(
        || state.translate(VirtAddr::new(block), effect.access(), CBOZ_BLOCK_SIZE),
        TranslateResult::Ready,
    );
    let (paddr, dirty_update) = match outcome {
        TranslateResult::Ready(r) => {
            if let Some(trap) = r.trap {
                push_trap(engine, ex, cbo::as_store_fault(trap, tval), ExceptionStage::Memory);
                return EntryOutcome::Done;
            }
            if r.cycles > 0 {
                let first = Some(TranslationResult { cycles: 0, ..r });
                return EntryOutcome::Delayed(DelayedAccess {
                    ready_cycle: state.cycle + r.cycles,
                    entry: ex,
                    translations: PageTranslations { first, second: None },
                });
            }
            (r.paddr, r.dirty_update)
        }
        TranslateResult::NeedPte { pte_addr, state: walk_state } => {
            park_walk(state, engine, walk_state, pte_addr, ex, PageTranslations::default());
            return EntryOutcome::ParkedWalk;
        }
    };

    let is_ram = state.bus.ram_region_for(paddr.val(), CBOZ_BLOCK_SIZE).is_some();
    if effect != cbo::CboEffect::Zero && !is_ram {
        push_trap(engine, ex, Trap::StoreAccessFault(tval), ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    let vaddr = VirtAddr::new(block);
    ex.alu = paddr.val();
    let dirty_updates = DirtyUpdates::of(dirty_update, None);
    engine
        .mem1_mem2_mut()
        .push(Mem1Mem2Entry { dirty_updates, ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr) });
    EntryOutcome::Done
}

/// True when `[paddr, paddr + size)` is not plain RAM: a device register,
/// or the HTIF window a device overlays.
fn reads_a_device(state: &StageCtx<'_>, paddr: PhysAddr, size: u64) -> bool {
    state.bus.ram_region_for(paddr.val(), size).is_none()
}

/// True when `tag` is the oldest instruction in the ROB and no squash on
/// its way will remove it.
fn is_rob_head<E: ExecutionEngine>(engine: &E, tag: RobTag) -> bool {
    engine.rob().peek_head().is_some_and(|head| head.tag == tag)
        && !engine.common().will_squash(tag)
}

/// Pushes an ALU/non-memory entry directly into the M1→M2 latch.
fn push_passthrough<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry) {
    let entry = Mem1Mem2Entry::from_execute(ex, VirtAddr::new(0), PhysAddr::new(0));
    engine.mem1_mem2_mut().push(entry);
}

/// Forwards an entry that already carries a trap from an earlier stage.
fn push_passthrough_with_trap<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry) {
    let vaddr = VirtAddr::new(ex.alu);
    engine.mem1_mem2_mut().push(Mem1Mem2Entry::from_execute(ex, vaddr, PhysAddr::new(0)));
}

/// Emits a fresh trap entry into the M1→M2 latch.
fn push_trap<E: ExecutionEngine>(
    engine: &mut E,
    ex: ExMem1Entry,
    trap: Trap,
    stage: ExceptionStage,
) {
    let vaddr = VirtAddr::new(ex.alu);
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        trap: Some(trap),
        exception_stage: Some(stage),
        ..Mem1Mem2Entry::from_execute(ex, vaddr, PhysAddr::new(0))
    });
}

/// Writes a translated scalar store's address and data into its
/// store-buffer slot, so younger loads can forward from it, and records
/// the oldest younger load that already read the location.
fn resolve_store<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    ex: &ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    outcome: &mut Memory1Outcome,
) {
    engine.store_buffer_mut().resolve(ex.rob_tag, vaddr, paddr, ex.store_data);
    outcome.resolved_stores.push(ex.rob_tag);
    let violator = engine
        .load_queue_mut()
        .and_then(|lq| lq.check_ordering_violation(paddr, ex.ctrl.width, ex.rob_tag));
    if let Some(load) = violator
        && outcome.violation.is_none_or(|(oldest, _)| load.is_older_than(oldest))
    {
        trace_fwd!(state.config.general.trace_instructions;
            event           = "violation",
            store_pc        = %crate::trace::Hex(ex.pc),
            store_tag       = ex.rob_tag.0,
            paddr           = %crate::trace::Hex(paddr.val()),
            violation_flush = load.0,
            "M1: memory ordering violation, a younger load already read this location"
        );
        outcome.violation = Some((load, ex.pc));
    }
    trace_mem!(state.config.general.trace_instructions;
        stage      = "M1",
        rob_tag    = ex.rob_tag.0,
        pc         = %crate::trace::Hex(ex.pc),
        op         = "store-resolve",
        paddr      = %crate::trace::Hex(paddr.val()),
        store_data = %crate::trace::Hex(ex.store_data),
        "M1: store resolved into store buffer (write deferred to commit)"
    );
}

/// Pushes a translated store or store-conditional into the M1→M2 latch.
/// A store-conditional resolves its store-buffer slot in memory2.
fn push_resolved_store<E: ExecutionEngine>(
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    dirty_updates: DirtyUpdates,
) {
    engine
        .mem1_mem2_mut()
        .push(Mem1Mem2Entry { dirty_updates, ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr) });
}

/// Pushes an SB-forwarded load into M1→M2 with the forwarded raw value
/// already in `load_data`.
fn push_sb_forwarded_load<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    dirty_updates: DirtyUpdates,
    raw_val: u64,
) {
    // Forwarded data still takes the load pipeline's time to arrive.
    let latency =
        if state.core().l1_d_cache.is_enabled() { state.core().l1_d_cache.latency } else { 1 };
    let entry = Mem1Mem2Entry {
        load_data: raw_val,
        sb_forwarded: true,
        dirty_updates,
        ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr)
    };
    engine
        .common_mut()
        .forwarded_loads
        .push(ForwardedLoad { ready_cycle: state.cycle + latency.max(1), entry });
}

/// Issues a `MemReq` for a load / LR / AMO and parks the entry.
fn emit_load_req<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    dirty_updates: DirtyUpdates,
    is_atomic: bool,
) {
    let access_size = match ex.ctrl.width {
        MemWidth::Byte => AccessSize::B1,
        MemWidth::Half => AccessSize::B2,
        MemWidth::Word => AccessSize::B4,
        MemWidth::Double | MemWidth::Nop => AccessSize::B8,
    };
    let op = if is_atomic {
        let packet_atomic = match ex.ctrl.atomic_op {
            AtomicOp::Lr => packet::AtomicOp::Lr,
            AtomicOp::Swap => packet::AtomicOp::Swap,
            AtomicOp::Add => packet::AtomicOp::Add,
            AtomicOp::Xor => packet::AtomicOp::Xor,
            AtomicOp::And => packet::AtomicOp::And,
            AtomicOp::Or => packet::AtomicOp::Or,
            AtomicOp::Min => packet::AtomicOp::Min,
            AtomicOp::Max => packet::AtomicOp::Max,
            AtomicOp::Minu => packet::AtomicOp::MinU,
            AtomicOp::Maxu => packet::AtomicOp::MaxU,
            AtomicOp::Sc => unreachable!("Sc is resolved at memory1 before emit"),
            AtomicOp::None => unreachable!("is_atomic checked"),
        };
        MemOp::Atomic { op: packet_atomic, data: ex.store_data }
    } else {
        MemOp::Read
    };

    let target = mmio_or_l1d(state, engine, paddr, access_size);
    let line_bytes = state.core().l1_d_cache.line_bytes() as u64;
    let second_line = (!matches!(target, ComponentId::Bus)
        && unaligned::crosses_cache_line(paddr.val(), ex.ctrl.width.bytes(), line_bytes))
    .then(|| PhysAddr::new((paddr.val() | (line_bytes - 1)) + 1));
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;

    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq { req_id, paddr, vaddr: Some(vaddr), size: access_size, op },
    );
    // The bytes past the line boundary are a second cache access.
    if let Some(second) = second_line {
        let common = engine.common_mut();
        let second_id = common.alloc_req_id();
        let _ = common.load_parts.insert(second_id, req_id);
        state.events().schedule(
            cycle,
            target,
            ComponentId::Pipeline(pipeline_id),
            Packet::MemReq {
                req_id: second_id,
                paddr: second,
                vaddr: Some(vaddr),
                size: access_size,
                op: MemOp::Read,
            },
        );
    }

    let side_effecting = matches!(target, ComponentId::Bus);
    let parts_outstanding = if second_line.is_some() { 2 } else { 1 };
    let _ = engine.common_mut().outstanding_loads.insert(
        req_id,
        OutstandingLoad {
            entry: ex,
            paddr,
            vaddr,
            dirty_updates,
            side_effecting,
            parts_outstanding,
        },
    );
}

/// Records the parked walk and issues the PTE `MemReq`.
fn park_walk<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    walk_state: crate::core::units::mmu::ptw::WalkState,
    pte_addr: PhysAddr,
    ex: ExMem1Entry,
    translations: PageTranslations,
) {
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let l1_d_id = common.l1_d_id;
    let pipeline_id = common.pipeline_id;
    let _ = common.outstanding_walks.insert(
        req_id,
        OutstandingWalk {
            state: walk_state,
            pte_addr,
            continuation: WalkContinuation::LoadStore {
                entry: ex,
                translations: Box::new(translations),
            },
        },
    );

    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        ComponentId::Cache(l1_d_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: pte_addr,
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Read,
        },
    );
}

/// Returns `ComponentId::Bus` when `paddr` is MMIO and `ComponentId::Cache(L1D)`
/// when it's pure RAM. MMIO loads/stores must bypass the L1D — caching MMIO
/// makes subsequent accesses hit the cache and silently miss the device's
/// side effect (e.g. an HTIF tohost write that L1D hit would never reach the
/// device).
fn mmio_or_l1d<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &E,
    paddr: PhysAddr,
    size: AccessSize,
) -> ComponentId {
    let size_bytes = match size {
        AccessSize::B1 => 1u64,
        AccessSize::B2 => 2,
        AccessSize::B4 => 4,
        AccessSize::B8 => 8,
        AccessSize::Line => 64,
    };
    if state.bus.ram_region_for(paddr.val(), size_bytes).is_some() {
        ComponentId::Cache(engine.common().l1_d_id)
    } else {
        ComponentId::Bus
    }
}

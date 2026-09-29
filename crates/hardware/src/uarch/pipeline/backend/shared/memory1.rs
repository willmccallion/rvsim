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
//!   - For **LR**: replay while any older store to the same address has not
//!     been written. Otherwise emit `MemReq` and park.
//!   - For **AMO / SC** (and an LR with `rl`), issued as the oldest
//!     instruction: replay until every older store has been written, set
//!     the PTE's D bit, then emit `MemReq` and park; the cache performs it.
//!   - For **stores**: pass to M1→M2 with the resolved `paddr`. Memory2
//!     resolves the store buffer and checks for ordering violations.

use crate::arch::translation::{DirtyUpdates, TranslationResult};
use crate::common::{AccessType, PhysAddr, VirtAddr, crosses_cache_line};
use crate::exec::cbo;
use crate::exec::compute::misaligned;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::op::{AtomicOp, MemWidth};
use crate::isa::privileged::Trap;
use crate::sim::StageCtx;
use crate::sim::components::ComponentId;
use crate::sim::packet::{self, AccessSize, MemOp, Packet};
use crate::sim::state::memory::TranslateResult;
use crate::sim::state::views::PteUpdateOutcome;
use crate::uarch::pipeline::engine::{ExecutionEngine, TrapProgress};
use crate::uarch::pipeline::exception::ExceptionStage;
use crate::uarch::pipeline::latches::{
    ExMem1Entry, Mem1Mem2Entry, MicroOpIdx, VecMemAccess, VecMemTarget,
};
use crate::uarch::pipeline::lsq::store_buffer::ForwardResult;
use crate::uarch::pipeline::lsq::vec_store_buffer::SpanForward;
use crate::uarch::pipeline::mailbox;
use crate::uarch::pipeline::outstanding::{
    DelayedAccess, ForwardedLoad, LoadParts, OutstandingLoad, OutstandingWalk, PageTranslations,
    WalkContinuation,
};
use crate::uarch::pipeline::rob::{RobState, RobTag};
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
    /// A vector span met a fault, a trigger or a device, which its elements
    /// must meet one by one.
    Expand(ExMem1Entry),
}

/// What memory1 resolved this cycle, for the engine to act on.
#[derive(Debug, Default)]
pub struct Memory1Outcome {
    /// Stores whose address and data entered the store buffer, in order.
    pub resolved_stores: Vec<RobTag>,
    /// The oldest younger load a resolving store found had already read
    /// the location, and that store's PC.
    pub violation: Option<(RobTag, u64)>,
    /// Vector spans to take apart into their elements.
    pub expanded_spans: Vec<ExMem1Entry>,
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
    let mut translations: Vec<(RobTag, Option<MicroOpIdx>, PageTranslations)> = ready
        .iter()
        .map(|a| {
            let micro_op = a.entry.vec_mem.as_ref().map(|v| v.micro_op);
            (a.entry.rob_tag, micro_op, a.translations.clone())
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
        let micro_op = ex.vec_mem.as_ref().map(|v| v.micro_op);
        let translated = translations
            .iter()
            .position(|(tag, m, _)| *tag == ex.rob_tag && *m == micro_op)
            .map(|i| translations.swap_remove(i).2)
            .unwrap_or_default();
        match process_entry(state, engine, ex, translated, &mut outcome) {
            EntryOutcome::Done => {}
            EntryOutcome::Replay(ex) => {
                state.counter(state.core().stat_paths.lsq.rescheduled_mem_ops).inc();
                engine.common_mut().mem1_replay.push(ex);
            }
            EntryOutcome::Delayed(access) => engine.common_mut().mem1_delayed.push(access),
            EntryOutcome::Expand(span) => outcome.expanded_spans.push(span),
            EntryOutcome::ParkedWalk => {
                input.extend(iter);
                return outcome;
            }
        }
    }
    outcome
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
        return translate_cbo(state, engine, ex, translated.first, resolved);
    }
    if ex.vec_mem.as_ref().is_some_and(|access| matches!(access.target, VecMemTarget::Span(_))) {
        return process_span(state, engine, ex, translated.first);
    }

    let needs_translation = ex.ctrl.mem_read || ex.ctrl.mem_write;
    if !needs_translation {
        push_passthrough(engine, ex);
        return EntryOutcome::Done;
    }

    // 2. Alignment.
    let size = misaligned::width_to_bytes(ex.ctrl.width);
    let is_atomic = ex.ctrl.atomic_op != AtomicOp::None;
    if !misaligned::is_aligned(ex.alu, size)
        && (state.config.memory.misaligned_access_trap || is_atomic)
    {
        let trap = if ex.ctrl.mem_write {
            misaligned::store_misaligned_trap(ex.alu)
        } else {
            misaligned::load_misaligned_trap(ex.alu)
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
    let (ex, first) =
        match translate_first_page(state, engine, ex, translated.first, access_type, size) {
            FirstPage::Translated(ex, first) => (ex, first),
            FirstPage::Waiting(waiting) => return waiting,
        };
    if let Some(trap) = first.trap {
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }
    let paddr = first.paddr;

    // 4b. A misaligned access that spills into the next page translates
    // that page as well; the two halves must then be physically adjacent
    // for the single-request data path, otherwise the access is left to
    // the misaligned trap handler like a real split-unaware LSU.
    let mut second_dirty_update = None;
    if let Some(second_va) = misaligned::second_page_start(ex.alu, size) {
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
                        misaligned::store_misaligned_trap(ex.alu)
                    } else {
                        misaligned::load_misaligned_trap(ex.alu)
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
        let micro_op = ex.vec_mem.as_ref().map(|v| v.micro_op);
        lq.fill_address(ex.rob_tag, micro_op, VirtAddr::new(ex.alu), paddr);
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
        if ex.ctrl.performs_at_rob_head() {
            // It follows every older store to memory, sets its PTE's D bit
            // as its translation completes, then takes effect in the cache.
            if !takes_effect_now(engine, ex.rob_tag) || older_stores_pending(state, engine) {
                return EntryOutcome::Replay(ex);
            }
            if !apply_dirty_updates(state, engine, dirty_updates) {
                return EntryOutcome::Replay(ex);
            }
            emit_load_req(state, engine, ex, paddr, vaddr, DirtyUpdates::NONE, true);
            return EntryOutcome::Done;
        }
        // LR: wait for older stores to this address to drain.
        if engine.store_buffer().has_older_store_to(paddr, ex.ctrl.width, ex.rob_tag)
            || engine.vec_store_buffer().has_older_store_to(paddr, size as usize, ex.rob_tag)
            || state.core_mut().wcb.request_send(paddr, size as usize)
        {
            return EntryOutcome::Replay(ex);
        }
        if reads_a_device(state, paddr, size) && !takes_effect_now(engine, ex.rob_tag) {
            return EntryOutcome::Replay(ex);
        }
        emit_load_req(state, engine, ex, paddr, vaddr, dirty_updates, true);
        return EntryOutcome::Done;
    }

    // A device read has side effects, so it waits until nothing older can
    // still fault, redirect or be interrupted: the load must be the oldest
    // instruction in the machine.
    if reads_a_device(state, paddr, size) && !takes_effect_now(engine, ex.rob_tag) {
        return EntryOutcome::Replay(ex);
    }

    match forward_from_pending_stores(state, engine, &ex, paddr, size as usize) {
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

/// How translating the page of an access's first byte came out.
enum FirstPage {
    /// The access with its translation, whose fault the caller raises.
    Translated(ExMem1Entry, TranslationResult),
    /// The access waits: out an L2 TLB hit's latency, or on a walk.
    Waiting(EntryOutcome),
}

/// Translates the page of `ex`'s first byte, unless `known` already has.
fn translate_first_page<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    known: Option<TranslationResult>,
    access_type: AccessType,
    size: u64,
) -> FirstPage {
    let outcome = known.map_or_else(
        || state.translate(VirtAddr::new(ex.alu), access_type, size),
        TranslateResult::Ready,
    );
    match outcome {
        TranslateResult::Ready(r) if r.trap.is_none() && r.cycles > 0 => {
            let first = Some(TranslationResult { cycles: 0, ..r });
            FirstPage::Waiting(EntryOutcome::Delayed(DelayedAccess {
                ready_cycle: state.cycle + r.cycles,
                entry: ex,
                translations: PageTranslations { first, second: None },
            }))
        }
        TranslateResult::Ready(r) => FirstPage::Translated(ex, r),
        TranslateResult::NeedPte { pte_addr, state: walk_state } => {
            park_walk(state, engine, walk_state, pte_addr, ex, PageTranslations::default());
            FirstPage::Waiting(EntryOutcome::ParkedWalk)
        }
    }
}

/// Processes a vector span: one translation, then one access for all its
/// elements. A span that meets a fault, a trigger or a device goes back to
/// be taken apart, so each element meets it on its own.
fn process_span<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    known: Option<TranslationResult>,
) -> EntryOutcome {
    let Some(access) = ex.vec_mem.as_ref() else { return EntryOutcome::Expand(ex) };
    let VecMemTarget::Span(span) = &access.target else { return EntryOutcome::Expand(ex) };
    let (is_store, micro_op, vaddr, bytes) =
        (access.is_store, access.micro_op, span.vaddr(), span.bytes());
    let triggered = span.elements.iter().any(|(_, element)| {
        let address = element.vaddr.val();
        if is_store {
            state.check_store_trigger(address)
        } else {
            state.check_load_trigger(address)
        }
    });
    if triggered {
        return EntryOutcome::Expand(ex);
    }

    let access_type = if is_store { AccessType::Write } else { AccessType::Read };
    let (ex, first) =
        match translate_first_page(state, engine, ex, known, access_type, bytes as u64) {
            FirstPage::Translated(ex, first) => (ex, first),
            FirstPage::Waiting(waiting) => return waiting,
        };
    let paddr = first.paddr;
    if first.trap.is_some() || reads_a_device(state, paddr, bytes as u64) {
        return EntryOutcome::Expand(ex);
    }
    let dirty_updates = DirtyUpdates::of(first.dirty_update, None);
    if is_store {
        push_resolved_store(engine, ex, paddr, vaddr, dirty_updates);
        return EntryOutcome::Done;
    }

    if let Some(lq) = engine.load_queue_mut() {
        lq.fill_address(ex.rob_tag, Some(micro_op), vaddr, paddr);
    }
    match forward_span_from_pending_stores(state, engine, &ex, paddr, bytes) {
        SpanForward::Hit(data) => {
            push_forwarded_span(state, engine, ex, paddr, vaddr, data);
            EntryOutcome::Done
        }
        SpanForward::Stall => EntryOutcome::Replay(ex),
        SpanForward::Miss => {
            emit_span_read(state, engine, ex, paddr, vaddr, bytes);
            EntryOutcome::Done
        }
    }
}

/// [`forward_from_pending_stores`] for a vector span: only a vector store
/// holding every byte forwards, and any other store it overlaps must be
/// written first.
fn forward_span_from_pending_stores<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &E,
    ex: &ExMem1Entry,
    paddr: PhysAddr,
    bytes: usize,
) -> SpanForward {
    if engine.store_buffer().overlaps_older_store(paddr, bytes, ex.rob_tag) {
        return SpanForward::Stall;
    }
    let vector = engine.vec_store_buffer().forward_span(paddr, bytes, ex.rob_tag);
    if state.core_mut().wcb.request_send(paddr, bytes) { SpanForward::Stall } else { vector }
}

/// Forwards to a load from the stores before it that have not reached the
/// cache: the store buffer's, which are the youngest, then the vector store
/// buffer's, then the write-combining buffer's. A load whose bytes both of
/// the last two hold waits, since their order is not known, and the WCB's
/// line is sent.
fn forward_from_pending_stores<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &E,
    ex: &ExMem1Entry,
    paddr: PhysAddr,
    bytes: usize,
) -> ForwardResult {
    let scalar = engine.store_buffer().forward_load(paddr, ex.ctrl.width, ex.rob_tag);
    if scalar != ForwardResult::Miss {
        return scalar;
    }
    let wcb = &mut state.core_mut().wcb;
    let vector = engine.vec_store_buffer().forward_load(paddr, ex.ctrl.width, ex.rob_tag);
    if vector == ForwardResult::Miss {
        return wcb.forward_load(paddr, bytes);
    }
    if wcb.request_send(paddr, bytes) { ForwardResult::Stall } else { vector }
}

/// Translates a cache-block operation's block and resolves it into its
/// store-buffer slot, which sends it to the cache after it commits. A fault
/// is reported as the store fault the CBO raises.
fn translate_cbo<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    translated: Option<TranslationResult>,
    resolved: &mut Memory1Outcome,
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

    // Device regions do not support cache-block operations, `cbo.zero`
    // included, which the CMO specification leaves to each I/O region.
    if state.bus.ram_region_for(paddr.val(), CBOZ_BLOCK_SIZE).is_none() {
        push_trap(engine, ex, Trap::StoreAccessFault(tval), ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    let vaddr = VirtAddr::new(block);
    resolve_block_op(engine, &ex, paddr, vaddr, effect, resolved);
    let dirty_updates = DirtyUpdates::of(dirty_update, None);
    engine
        .mem1_mem2_mut()
        .push(Mem1Mem2Entry { dirty_updates, ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr) });
    EntryOutcome::Done
}

/// Puts a translated CBO in its store-buffer slot, where it is ordered as a
/// store, and records the oldest younger load that already read its block.
fn resolve_block_op<E: ExecutionEngine>(
    engine: &mut E,
    ex: &ExMem1Entry,
    block: PhysAddr,
    vaddr: VirtAddr,
    effect: cbo::CboEffect,
    outcome: &mut Memory1Outcome,
) {
    engine.store_buffer_mut().resolve_block(ex.rob_tag, vaddr, block, effect);
    outcome.resolved_stores.push(ex.rob_tag);
    let violator = engine
        .load_queue_mut()
        .and_then(|lq| lq.check_ordering_violation_over(block, CBOZ_BLOCK_SIZE, ex.rob_tag));
    if let Some(load) = violator
        && outcome.violation.is_none_or(|(oldest, _)| load.is_older_than(oldest))
    {
        outcome.violation = Some((load, ex.pc));
    }
}

/// True while a committed store has yet to finish writing.
fn older_stores_pending<E: ExecutionEngine>(state: &StageCtx<'_>, engine: &E) -> bool {
    engine.store_buffer().has_committed_stores()
        || engine.vec_store_buffer().has_committed_stores()
        || state.core().wcb.has_pending()
}

/// Sets the D bits an atomic's translation needs before it takes effect.
/// Returns false when a PTE changed since its walk, so the access must
/// translate again.
fn apply_dirty_updates<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    updates: DirtyUpdates,
) -> bool {
    for update in updates.iter() {
        match state.apply_pte_update(update) {
            PteUpdateOutcome::Changed => return false,
            PteUpdateOutcome::Written(pte) => {
                mailbox::send_pte_write(engine.common_mut(), state, update.pte_addr, pte);
            }
            PteUpdateOutcome::AlreadySet => {}
        }
    }
    true
}

/// True when `[paddr, paddr + size)` is not plain RAM: a device register,
/// or the HTIF window a device overlays.
fn reads_a_device(state: &StageCtx<'_>, paddr: PhysAddr, size: u64) -> bool {
    state.bus.ram_region_for(paddr.val(), size).is_none()
}

/// True when an access that has an effect nothing can undo may take it:
/// `tag` is the oldest instruction and nothing on its way will remove it,
/// neither a squash nor a trap, whose flush takes everything in flight.
fn takes_effect_now<E: ExecutionEngine>(engine: &E, tag: RobTag) -> bool {
    let common = engine.common();
    engine.rob().peek_head().is_some_and(|head| head.tag == tag)
        && !common.will_squash(tag)
        && !matches!(common.trap, TrapProgress::Pending(_))
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

/// Parks a vector span a store forwarded, which arrives after the load
/// pipeline's latency like a scalar forward.
fn push_forwarded_span<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    mut ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    data: Box<[u8]>,
) {
    if let Some(VecMemAccess { target: VecMemTarget::Span(span), .. }) = ex.vec_mem.as_mut() {
        span.data = Some(data);
    }
    let latency =
        if state.core().l1_d_cache.is_enabled() { state.core().l1_d_cache.latency } else { 1 };
    let entry =
        Mem1Mem2Entry { sb_forwarded: true, ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr) };
    engine
        .common_mut()
        .forwarded_loads
        .push(ForwardedLoad { ready_cycle: state.cycle + latency.max(1), entry });
}

/// Issues a vector span's read of its `bytes` bytes to the L1D and parks it.
fn emit_span_read<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    bytes: usize,
) {
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let target = ComponentId::Cache(common.l1_d_id);
    let pipeline = ComponentId::Pipeline(common.pipeline_id);
    let size = AccessSize::Span(bytes as u8);
    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        target,
        pipeline,
        Packet::MemReq { req_id, paddr, vaddr: Some(vaddr), size, op: MemOp::Read },
    );
    let _ = engine.common_mut().outstanding_loads.insert(
        req_id,
        OutstandingLoad {
            entry: ex,
            paddr,
            vaddr,
            dirty_updates: DirtyUpdates::NONE,
            side_effecting: false,
            parts: LoadParts::Whole(None),
        },
    );
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
            AtomicOp::Sc => packet::AtomicOp::Sc,
            AtomicOp::None => unreachable!("is_atomic checked"),
        };
        MemOp::Atomic { op: packet_atomic, data: ex.store_data, hart: state.hart().hart_id }
    } else {
        MemOp::Read
    };

    let target = mmio_or_l1d(state, engine, paddr, access_size);
    let line_bytes = state.core().l1_d_cache.line_bytes() as u64;
    let width_bytes = ex.ctrl.width.bytes();
    let second_line = (!matches!(target, ComponentId::Bus)
        && crosses_cache_line(paddr.val(), width_bytes, line_bytes))
    .then(|| PhysAddr::new((paddr.val() | (line_bytes - 1)) + 1));
    let low_bytes = second_line.map(|second| second.val() - paddr.val());
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;

    let cycle = state.cycle;
    let first_size = low_bytes.map_or(access_size, |low| AccessSize::of_bytes(low as usize));
    state.events().schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq { req_id, paddr, vaddr: Some(vaddr), size: first_size, op },
    );
    // The bytes past the line boundary are a second cache access.
    if let (Some(second), Some(low)) = (second_line, low_bytes) {
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
                size: AccessSize::of_bytes((width_bytes - low) as usize),
                op: MemOp::Read,
            },
        );
    }

    let side_effecting = matches!(target, ComponentId::Bus) || ex.ctrl.performs_at_rob_head();
    let parts = low_bytes.map_or(LoadParts::Whole(None), |low| LoadParts::Split {
        low_bytes: low as u8,
        low: None,
        high: None,
    });
    let _ = engine.common_mut().outstanding_loads.insert(
        req_id,
        OutstandingLoad { entry: ex, paddr, vaddr, dirty_updates, side_effecting, parts },
    );
}

/// Records the parked walk and issues the PTE `MemReq`.
fn park_walk<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    walk_state: crate::uarch::mmu::ptw::WalkState,
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
    if state.bus.ram_region_for(paddr.val(), size.bytes() as u64).is_some() {
        ComponentId::Cache(engine.common().l1_d_id)
    } else {
        ComponentId::Bus
    }
}

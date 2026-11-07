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
//!   - On a [`TranslateOutcome::NeedPte`], park the entry under an
//!     [`OutstandingWalk`] with [`WalkContinuation::LoadStore`] and emit a
//!     `MemReq` for the PTE. When the walk completes the mailbox drain
//!     hands the entry back here with the walk's translation.
//!   - On a fault (PMP / page fault / unmapped paddr), emit a trapped
//!     `Mem1Mem2Entry`.
//!   - For demand **loads**: check store-buffer forwarding first.
//!     - SB hit → the load reaches M1→M2 with `load_data` filled and
//!       `sb_forwarded = true` after the forwarding latency, or at zero
//!       latency is finalized here and written back this cycle. No
//!       `MemReq` issued.
//!     - SB partial overlap → move to `BackendCommon::mem1_replay` and retry
//!       next cycle; the store it overlaps must drain first.
//!     - SB miss → emit `MemReq` to L1D and park [`OutstandingLoad`].
//!   - For **LR**: replay while any older store to the same address has not
//!     been written. Otherwise emit `MemReq` and park.
//!   - For a **device load**: replay until it is the oldest instruction and
//!     every older store to its bytes has been written, then emit `MemReq`
//!     to the device. It never forwards from a buffered store.
//!   - For **AMO / SC** (and an LR with `rl`), issued as the oldest
//!     instruction: replay until every older store has been written, set
//!     the PTE's D bit, then emit `MemReq` and park; the cache performs it.
//!   - For **stores**: pass to M1→M2 with the resolved `paddr`. Memory2
//!     resolves the store buffer and checks for ordering violations.

use std::ops::ControlFlow;

use crate::arch::translation::{DirtyUpdates, PteUpdate, TranslationResult};
use crate::common::{AccessType, PhysAddr, VirtAddr, crosses_cache_line};
use crate::exec::cbo;
use crate::exec::compute::misaligned;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::op::MemWidth;
use crate::isa::privileged::Trap;
use crate::sim::components::ComponentId;
use crate::sim::packet::{AccessSize, MemOp, Packet};
use crate::uarch::ctx::StageCtx;
use crate::uarch::ctx::stage::PteUpdateOutcome;
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::pipeline::backend::shared::memory2;
use crate::uarch::pipeline::engine::{ExecutionEngine, TrapProgress};
use crate::uarch::pipeline::latches::{
    ExMem1Entry, Mem1Mem2Entry, MicroOpIdx, VecMemAccess, VecMemTarget,
};
use crate::uarch::pipeline::lsq::load_queue::OrderViolation;
use crate::uarch::pipeline::lsq::store_buffer::ForwardResult;
use crate::uarch::pipeline::lsq::vec_store_buffer::SpanForward;
use crate::uarch::pipeline::mailbox;
use crate::uarch::pipeline::outstanding::{
    DelayedAccess, ForwardedLoad, LoadParts, OutstandingLoad, OutstandingWalk, PageTranslations,
    WalkContinuation,
};
use crate::uarch::pipeline::rob::{RobEntry, RobTag};
use crate::uarch::pipeline::trace::Stage;
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
    /// the location.
    pub violation: Option<OrderViolation>,
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
    let mut entries = std::mem::take(&mut engine.common_mut().memory.mem1_replay);
    entries.append(input);
    let delayed = &mut engine.common_mut().memory.mem1_delayed;
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
            let micro_op = a.entry.op.vec_mem.as_ref().map(|v| v.micro_op);
            (a.entry.op.rob_tag, micro_op, a.translations.clone())
        })
        .collect();
    entries.extend(ready.into_iter().map(|a| a.entry));
    // Out-of-order execute can drop entries into execute_mem1 in completion
    // order rather than program order. memory1's SB-forward / atomic-vs-SB
    // checks only inspect *older* store entries, so process oldest first to
    // give each op the most-drained store buffer view available this cycle.
    entries.sort_by(|a, b| a.op.rob_tag.age_cmp(b.op.rob_tag));
    let mut iter = entries.into_iter();

    while let Some(mut ex) = iter.next() {
        if let Some(seq) = engine.rob().find_entry(ex.op.rob_tag).map(|entry| entry.seq) {
            state.trace_stage(seq, Stage::Memory);
        }
        let was_waiting = std::mem::take(&mut ex.replaying);
        let micro_op = ex.op.vec_mem.as_ref().map(|v| v.micro_op);
        let translated = translations
            .iter()
            .position(|(tag, m, _)| *tag == ex.op.rob_tag && *m == micro_op)
            .map(|i| translations.swap_remove(i).2)
            .unwrap_or_default();
        match process_entry(state, engine, ex, translated, &mut outcome) {
            EntryOutcome::Done => {}
            EntryOutcome::Replay(mut ex) => {
                if !was_waiting {
                    state.counter(state.core().stat_paths.lsq.rescheduled_mem_ops).inc();
                }
                ex.replaying = true;
                engine.common_mut().memory.mem1_replay.push(ex);
            }
            EntryOutcome::Delayed(access) => engine.common_mut().memory.mem1_delayed.push(access),
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
    let Some(ex) = pass_on_fault(engine, ex) else {
        return EntryOutcome::Done;
    };
    if ex.op.ctrl.system_op.is_cbo() {
        return translate_cbo(state, engine, ex, translated.first, resolved);
    }
    if ex.op.vec_mem.as_ref().is_some_and(|access| matches!(access.target, VecMemTarget::Span(_))) {
        return process_span(state, engine, ex, translated.first);
    }
    if !ex.op.ctrl.mem_read && !ex.op.ctrl.mem_write {
        push_passthrough(engine, ex);
        return EntryOutcome::Done;
    }
    if let Err(trap) = check_access(state, &ex) {
        push_trap(engine, ex, trap);
        return EntryOutcome::Done;
    }
    let access = match translate_access(state, engine, ex, translated) {
        ControlFlow::Continue(access) => access,
        ControlFlow::Break(outcome) => return outcome,
    };
    if access.ex.op.ctrl.mem_read
        && let Some(lq) = engine.load_queue_mut()
    {
        let micro_op = access.ex.op.vec_mem.as_ref().map(|v| v.micro_op);
        lq.fill_address(access.ex.op.rob_tag, micro_op, access.ex.vaddr(), access.paddr);
    }
    if access.ex.op.ctrl.atomic_op.is_some() {
        return perform_atomic(state, engine, access);
    }
    if access.ex.op.ctrl.mem_write {
        perform_store(state, engine, access, resolved);
        return EntryOutcome::Done;
    }
    perform_load(state, engine, access)
}

/// `ex`, unless it or its ROB entry already faulted: then it carries the
/// trap on from here and performs no access.
fn pass_on_fault<E: ExecutionEngine>(engine: &mut E, mut ex: ExMem1Entry) -> Option<ExMem1Entry> {
    if ex.op.trap.is_none() {
        let Some(trap) = engine.rob().find_entry(ex.op.rob_tag).and_then(RobEntry::trap) else {
            return Some(ex);
        };
        ex.op.trap = Some(trap.clone());
    }
    push_passthrough_with_trap(engine, ex);
    None
}

/// The fault an access raises before it is translated: misalignment where
/// that traps, then an Sdtrig load or store trigger on its address, whose
/// breakpoint's tval is the address.
fn check_access(state: &StageCtx<'_>, ex: &ExMem1Entry) -> Result<(), Trap> {
    let is_atomic = ex.op.ctrl.atomic_op.is_some();
    let traps_misaligned = state.config.memory.misaligned_access_trap || is_atomic;
    if traps_misaligned && !misaligned::is_aligned(ex.op.alu, ex.op.ctrl.width.bytes()) {
        return Err(misaligned_trap(ex));
    }
    let is_triggered = !is_atomic
        && ((ex.op.ctrl.mem_read && state.check_load_trigger(ex.op.alu))
            || (ex.op.ctrl.mem_write && state.check_store_trigger(ex.op.alu)));
    if is_triggered {
        return Err(Trap::Breakpoint(ex.op.alu));
    }
    Ok(())
}

/// The misaligned-address fault `ex` raises, a load's or a store's.
const fn misaligned_trap(ex: &ExMem1Entry) -> Trap {
    if ex.op.ctrl.mem_write {
        misaligned::store_misaligned_trap(ex.op.alu)
    } else {
        misaligned::load_misaligned_trap(ex.op.alu)
    }
}

/// An access with its translation: the physical address of its first
/// byte, and the D-bit updates a store makes as it retires.
struct TranslatedAccess {
    ex: ExMem1Entry,
    paddr: PhysAddr,
    dirty_updates: DirtyUpdates,
}

/// Translates `ex`, unless `known` already has, and the next page too when
/// a misaligned access spills into it. Breaks with the outcome when the
/// translation faults or the access has to wait for it.
fn translate_access<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    known: PageTranslations,
) -> ControlFlow<EntryOutcome, TranslatedAccess> {
    let (access_type, size) = (access_type(&ex), ex.op.ctrl.width.bytes());
    let (ex, first) = match translate_first_page(state, engine, ex, known.first, access_type, size)
    {
        FirstPage::Translated(ex, first) => (ex, first),
        FirstPage::Waiting(waiting) => return ControlFlow::Break(waiting),
    };
    if let Some(trap) = first.trap {
        push_trap(engine, ex, trap);
        return ControlFlow::Break(EntryOutcome::Done);
    }
    let (ex, second_dirty_update) = translate_second_page(state, engine, ex, &first, known.second)?;
    let dirty_updates = DirtyUpdates::of(first.dirty_update, second_dirty_update);
    ControlFlow::Continue(TranslatedAccess { ex, paddr: first.paddr, dirty_updates })
}

/// The read or write `ex` makes of memory.
const fn access_type(ex: &ExMem1Entry) -> AccessType {
    if ex.op.ctrl.mem_write { AccessType::Write } else { AccessType::Read }
}

/// How translating the page of an access's first byte came out.
enum FirstPage {
    /// The access with its translation, whose fault the caller raises.
    Translated(ExMem1Entry, TranslationResult),
    /// The access waits: out an L2 TLB hit's latency, or on a walk.
    Waiting(EntryOutcome),
}

/// Translates the page of `ex`'s first byte, unless `known` already has.
/// An L2 TLB hit refills the L1 and costs its latency before the access
/// continues; the L1 TLB answers in the same cycle.
fn translate_first_page<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    known: Option<TranslationResult>,
    access_type: AccessType,
    size: u64,
) -> FirstPage {
    let outcome = known
        .map_or_else(|| state.translate(ex.vaddr(), access_type, size), TranslateOutcome::Ready);
    match outcome {
        TranslateOutcome::Ready(r) if r.trap.is_none() && r.cycles > 0 => {
            let cycles = r.cycles;
            let translations = PageTranslations { first: Some(r.settled()), second: None };
            FirstPage::Waiting(wait_for_l2_tlb(state, ex, translations, cycles))
        }
        TranslateOutcome::Ready(r) => FirstPage::Translated(ex, r),
        TranslateOutcome::NeedPte { pte_addr, state: walk_state } => {
            let translations = PageTranslations::default();
            park_walk(state, engine, walk_state, pte_addr, size, ex, translations);
            FirstPage::Waiting(EntryOutcome::ParkedWalk)
        }
    }
}

/// `ex` with the D-bit update of the next page, which a misaligned access
/// spilling into it translates as well, unless `known` already has. The
/// two halves must be physically adjacent for the single-request data
/// path; otherwise the access is left to the misaligned trap handler, like
/// a real split-unaware LSU.
fn translate_second_page<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    first: &TranslationResult,
    known: Option<TranslationResult>,
) -> ControlFlow<EntryOutcome, (ExMem1Entry, Option<PteUpdate>)> {
    let Some(second_va) = misaligned::second_page_start(ex.op.alu, ex.op.ctrl.width.bytes()) else {
        return ControlFlow::Continue((ex, None));
    };
    let outcome = known.map_or_else(
        || state.translate(VirtAddr::new(second_va), access_type(&ex), 1),
        TranslateOutcome::Ready,
    );
    let r = match outcome {
        TranslateOutcome::Ready(r) => r,
        TranslateOutcome::NeedPte { pte_addr, state: walk_state } => {
            let translations = PageTranslations { first: Some(first.clone()), second: None };
            park_walk(state, engine, walk_state, pte_addr, 1, ex, translations);
            return ControlFlow::Break(EntryOutcome::ParkedWalk);
        }
    };
    if let Some(trap) = r.trap {
        push_trap(engine, ex, trap);
        return ControlFlow::Break(EntryOutcome::Done);
    }
    if r.cycles > 0 {
        let cycles = r.cycles;
        let translations =
            PageTranslations { first: Some(first.clone()), second: Some(r.settled()) };
        return ControlFlow::Break(wait_for_l2_tlb(state, ex, translations, cycles));
    }
    let first_page_bytes = second_va.wrapping_sub(ex.op.alu);
    if r.paddr.val() != first.paddr.val().wrapping_add(first_page_bytes) {
        let trap = misaligned_trap(&ex);
        push_trap(engine, ex, trap);
        return ControlFlow::Break(EntryOutcome::Done);
    }
    ControlFlow::Continue((ex, r.dirty_update))
}

/// `ex` waiting out the `cycles` an L2 TLB hit costs, to continue with
/// `translations`.
fn wait_for_l2_tlb(
    state: &StageCtx<'_>,
    ex: ExMem1Entry,
    translations: PageTranslations,
    cycles: u64,
) -> EntryOutcome {
    EntryOutcome::Delayed(DelayedAccess {
        ready_cycle: state.cycle + cycles,
        entry: ex,
        translations,
    })
}

/// Resolves a plain store: a scalar one into its store-buffer slot, and
/// either on to memory2.
fn perform_store<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    access: TranslatedAccess,
    resolved: &mut Memory1Outcome,
) {
    let TranslatedAccess { ex, paddr, dirty_updates } = access;
    let vaddr = ex.vaddr();
    if ex.op.vec_mem.is_none() {
        resolve_store(state, engine, &ex, paddr, vaddr, resolved);
    }
    push_resolved_store(engine, ex, paddr, vaddr, dirty_updates);
}

/// Sends an LR, SC or AMO to the cache, replaying it until it may go.
fn perform_atomic<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    access: TranslatedAccess,
) -> EntryOutcome {
    let TranslatedAccess { ex, paddr, dirty_updates } = access;
    let vaddr = ex.vaddr();
    if ex.op.ctrl.performs_at_rob_head() {
        // It follows every older store to memory, sets its PTE's D bit
        // as its translation completes, then takes effect in the cache.
        if !takes_effect_now(engine, ex.op.rob_tag) || older_stores_pending(state, engine) {
            return EntryOutcome::Replay(ex);
        }
        if !apply_dirty_updates(state, engine, dirty_updates) {
            return EntryOutcome::Replay(ex);
        }
        emit_load_req(state, engine, ex, paddr, vaddr, DirtyUpdates::NONE);
        return EntryOutcome::Done;
    }
    // LR: wait for older stores to this address to drain.
    let size = ex.op.ctrl.width.bytes();
    if older_store_unwritten(state, engine, &ex, paddr, size) {
        return EntryOutcome::Replay(ex);
    }
    if reads_a_device(state, paddr, size) && !takes_effect_now(engine, ex.op.rob_tag) {
        return EntryOutcome::Replay(ex);
    }
    emit_load_req(state, engine, ex, paddr, vaddr, dirty_updates);
    EntryOutcome::Done
}

/// Sends a load to a device or the cache, or forwards it from the store
/// buffer, replaying it until it may go.
fn perform_load<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    access: TranslatedAccess,
) -> EntryOutcome {
    let TranslatedAccess { ex, paddr, dirty_updates } = access;
    let (pc, vaddr, size) = (ex.op.pc, ex.vaddr(), ex.op.ctrl.width.bytes());
    // A device read has side effects, so it waits until nothing older can
    // still fault, redirect or be interrupted: the load must be the oldest
    // instruction in the machine. A register need not read back what was
    // written to it, so the read goes to the device after the stores to it.
    if reads_a_device(state, paddr, size) {
        if !takes_effect_now(engine, ex.op.rob_tag)
            || older_store_unwritten(state, engine, &ex, paddr, size)
        {
            return EntryOutcome::Replay(ex);
        }
        emit_load_req(state, engine, ex, paddr, vaddr, dirty_updates);
        send_load_prefetches(state, engine, pc, vaddr, paddr);
        return EntryOutcome::Done;
    }
    match forward_from_pending_stores(state, engine, &ex, paddr, size as usize) {
        ForwardResult::Hit(raw_val) => {
            push_sb_forwarded_load(state, engine, ex, paddr, vaddr, dirty_updates, raw_val);
        }
        ForwardResult::Stall => return EntryOutcome::Replay(ex),
        ForwardResult::Miss => emit_load_req(state, engine, ex, paddr, vaddr, dirty_updates),
    }
    send_load_prefetches(state, engine, pc, vaddr, paddr);
    EntryOutcome::Done
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
    let Some(access) = ex.op.vec_mem.as_ref() else { return EntryOutcome::Expand(ex) };
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
        lq.fill_address(ex.op.rob_tag, Some(micro_op), vaddr, paddr);
    }
    let pc = ex.op.pc;
    match forward_span_from_pending_stores(state, engine, &ex, paddr, bytes) {
        SpanForward::Hit(data) => {
            push_forwarded_span(state, engine, ex, paddr, vaddr, data);
            send_load_prefetches(state, engine, pc, vaddr, paddr);
            EntryOutcome::Done
        }
        SpanForward::Stall => EntryOutcome::Replay(ex),
        SpanForward::Miss => {
            emit_span_read(state, engine, ex, paddr, vaddr, bytes);
            send_load_prefetches(state, engine, pc, vaddr, paddr);
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
    if engine.store_buffer().overlaps_older_store(paddr, bytes, ex.op.rob_tag) {
        return SpanForward::Stall;
    }
    let vector = engine.vec_store_buffer().forward_span(paddr, bytes, ex.op.rob_tag);
    if state.core_mut().wcb.request_send(paddr, bytes) { SpanForward::Stall } else { vector }
}

/// Forwards to a load from the stores before it that have not reached the
/// cache. The youngest older store in the store buffer or the vector store
/// buffer that writes any of its bytes decides; failing both, the
/// write-combining buffer's, which hold older stores. A load whose bytes a
/// vector store and the WCB both hold waits, since their order is not
/// known, and the WCB's line is sent.
fn forward_from_pending_stores<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &E,
    ex: &ExMem1Entry,
    paddr: PhysAddr,
    bytes: usize,
) -> ForwardResult {
    let store_buffer = engine.store_buffer();
    let scalar = store_buffer.youngest_older_store_to(paddr, ex.op.ctrl.width, ex.op.rob_tag);
    let vector_is_younger = scalar.is_some_and(|scalar| {
        engine.vec_store_buffer().writes_to_between(scalar, ex.op.rob_tag, paddr, bytes)
    });
    if scalar.is_some() && !vector_is_younger {
        return store_buffer.forward_load(paddr, ex.op.ctrl.width, ex.op.rob_tag);
    }
    let wcb = &mut state.core_mut().wcb;
    let vector = engine.vec_store_buffer().forward_load(paddr, ex.op.ctrl.width, ex.op.rob_tag);
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
    let effect = match cbo::gate(&hart.csrs, hart.privilege, ex.op.ctrl.system_op, ex.op.bits) {
        Ok(effect) => effect,
        Err(trap) => {
            push_trap(engine, ex, trap);
            return EntryOutcome::Done;
        }
    };
    let rs1 = ex.op.alu;
    let block = cbo::block_address(rs1);
    let tval = cbo::fault_address(ex.op.ctrl.system_op, rs1);
    if state.check_store_trigger(block) {
        let trap = Trap::Breakpoint(tval);
        push_trap(engine, ex, trap);
        return EntryOutcome::Done;
    }

    let outcome = translated.map_or_else(
        || state.translate(VirtAddr::new(block), effect.access(), CBOZ_BLOCK_SIZE),
        TranslateOutcome::Ready,
    );
    let (paddr, dirty_update) = match outcome {
        TranslateOutcome::Ready(r) => {
            if let Some(trap) = r.trap {
                push_trap(engine, ex, cbo::as_store_fault(trap, tval));
                return EntryOutcome::Done;
            }
            if r.cycles > 0 {
                let cycles = r.cycles;
                let translations = PageTranslations { first: Some(r.settled()), second: None };
                return wait_for_l2_tlb(state, ex, translations, cycles);
            }
            (r.paddr, r.dirty_update)
        }
        TranslateOutcome::NeedPte { pte_addr, state: walk_state } => {
            let translations = PageTranslations::default();
            park_walk(state, engine, walk_state, pte_addr, CBOZ_BLOCK_SIZE, ex, translations);
            return EntryOutcome::ParkedWalk;
        }
    };

    // Device regions do not support cache-block operations, `cbo.zero`
    // included, which the CMO specification leaves to each I/O region.
    if !state.bus.is_ram(paddr, CBOZ_BLOCK_SIZE) {
        push_trap(engine, ex, Trap::StoreAccessFault(tval));
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
    engine.store_buffer_mut().resolve_block(ex.op.rob_tag, vaddr, block, effect);
    outcome.resolved_stores.push(ex.op.rob_tag);
    let violation = engine
        .load_queue_mut()
        .and_then(|lq| lq.check_ordering_violation_over(block, CBOZ_BLOCK_SIZE, ex.op.rob_tag))
        .map(|load| OrderViolation { load, store_pc: ex.op.pc });
    outcome.violation = OrderViolation::older(outcome.violation, violation);
}

/// True while a store older than `ex` that writes any of the `size` bytes
/// at `paddr` has not been written; a WCB line holding them is sent.
fn older_store_unwritten<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &E,
    ex: &ExMem1Entry,
    paddr: PhysAddr,
    size: u64,
) -> bool {
    engine.store_buffer().has_older_store_to(paddr, ex.op.ctrl.width, ex.op.rob_tag)
        || engine.vec_store_buffer().has_older_store_to(paddr, size as usize, ex.op.rob_tag)
        || state.core_mut().wcb.request_send(paddr, size as usize)
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
    !state.bus.is_ram(paddr, size)
}

/// True when an access that has an effect nothing can undo may take it:
/// `tag` was the oldest instruction as the cycle began, and nothing on its
/// way will remove it, neither a squash nor a trap, whose flush takes
/// everything in flight.
fn takes_effect_now<E: ExecutionEngine>(engine: &E, tag: RobTag) -> bool {
    let common = engine.common();
    common.commit.head_at_cycle_start.is(tag)
        && !common.will_squash(tag)
        && !matches!(common.commit.trap, TrapProgress::Pending(_))
}

/// Pushes an ALU/non-memory entry directly into the M1→M2 latch.
fn push_passthrough<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry) {
    let entry = Mem1Mem2Entry::from_execute(ex, VirtAddr::new(0), PhysAddr::new(0));
    engine.mem1_mem2_mut().push(entry);
}

/// Forwards an entry that already carries a trap from an earlier stage.
fn push_passthrough_with_trap<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry) {
    let vaddr = ex.vaddr();
    engine.mem1_mem2_mut().push(Mem1Mem2Entry::from_execute(ex, vaddr, PhysAddr::new(0)));
}

/// Emits a fresh trap entry into the M1→M2 latch.
fn push_trap<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry, trap: Trap) {
    let vaddr = ex.vaddr();
    let mut entry = Mem1Mem2Entry::from_execute(ex, vaddr, PhysAddr::new(0));
    entry.op.trap = Some(trap);
    engine.mem1_mem2_mut().push(entry);
}

/// Writes a translated scalar store's address, and its data unless its
/// data half delivers that separately, into its store-buffer slot, so
/// younger loads can forward from it, and records the oldest younger load
/// that already read the location.
fn resolve_store<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    ex: &ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    outcome: &mut Memory1Outcome,
) {
    if ex.store_data_follows {
        engine.store_buffer_mut().resolve_address(ex.op.rob_tag, vaddr, paddr);
    } else {
        engine.store_buffer_mut().resolve(ex.op.rob_tag, vaddr, paddr, ex.store_data);
    }
    outcome.resolved_stores.push(ex.op.rob_tag);
    let violation = engine
        .load_queue_mut()
        .and_then(|lq| lq.check_ordering_violation(paddr, ex.op.ctrl.width, ex.op.rob_tag))
        .map(|load| OrderViolation { load, store_pc: ex.op.pc });
    if let Some(found) = violation
        && found.precedes(outcome.violation)
    {
        trace_fwd!(state.config.general.trace_instructions;
            event           = "violation",
            store_pc        = %crate::common::trace::Hex(ex.op.pc.val()),
            store_tag       = %ex.op.rob_tag,
            paddr           = %crate::common::trace::Hex(paddr.val()),
            violation_flush = %found.load,
            "M1: memory ordering violation, a younger load already read this location"
        );
        outcome.violation = Some(found);
    }
    trace_mem!(state.config.general.trace_instructions;
        stage      = "M1",
        rob_tag    = %ex.op.rob_tag,
        pc         = %crate::common::trace::Hex(ex.op.pc.val()),
        op         = "store-resolve",
        paddr      = %crate::common::trace::Hex(paddr.val()),
        store_data = %crate::common::trace::Hex(ex.store_data),
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

/// Cycles a forwarded load's data takes to reach writeback: the configured
/// store-forward latency, else the L1D hit latency it would have paid.
fn forward_latency(state: &StageCtx<'_>) -> u64 {
    state.config.pipeline.store_forward_latency.unwrap_or_else(|| {
        let l1_d = &state.core().l1_d_cache;
        if l1_d.is_enabled() { l1_d.latency.max(1) } else { 1 }
    })
}

/// Completes an SB-forwarded load with the forwarded raw value in
/// `load_data`: finalized and written back this cycle at zero latency,
/// otherwise reaching memory2 after the forwarding latency.
fn push_sb_forwarded_load<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    dirty_updates: DirtyUpdates,
    raw_val: u64,
) {
    let entry = Mem1Mem2Entry {
        load_data: raw_val,
        sb_forwarded: true,
        dirty_updates,
        ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr)
    };
    let latency = forward_latency(state);
    if latency == 0 {
        let result = memory2::finalize_load(state, entry, engine.load_queue_mut(), "M1");
        engine.common_mut().memory.forwarded_results.push(result);
        return;
    }
    engine
        .common_mut()
        .memory
        .forwarded_loads
        .push(ForwardedLoad { ready_cycle: state.cycle + latency, entry });
}

/// Parks a vector span a store forwarded, which arrives after the
/// forwarding latency like a scalar forward; its elements are taken apart
/// in memory2, so it takes at least one cycle.
fn push_forwarded_span<E: ExecutionEngine>(
    state: &StageCtx<'_>,
    engine: &mut E,
    mut ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    data: Box<[u8]>,
) {
    if let Some(VecMemAccess { target: VecMemTarget::Span(span), .. }) = ex.op.vec_mem.as_mut() {
        span.data = Some(data);
    }
    let latency = forward_latency(state).max(1);
    let entry =
        Mem1Mem2Entry { sb_forwarded: true, ..Mem1Mem2Entry::from_execute(ex, vaddr, paddr) };
    engine
        .common_mut()
        .memory
        .forwarded_loads
        .push(ForwardedLoad { ready_cycle: state.cycle + latency, entry });
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
    let pc = ex.op.pc;
    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        target,
        pipeline,
        Packet::MemReq { req_id, paddr, vaddr: Some(vaddr), pc: Some(pc), size, op: MemOp::Read },
    );
    let _ = engine.common_mut().memory.outstanding_loads.insert(
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
) {
    let access_size = match ex.op.ctrl.width {
        MemWidth::Byte => AccessSize::B1,
        MemWidth::Half => AccessSize::B2,
        MemWidth::Word => AccessSize::B4,
        MemWidth::Double | MemWidth::Nop => AccessSize::B8,
    };
    let op = match ex.op.ctrl.atomic_op {
        Some(op) => MemOp::Atomic { op, data: ex.store_data, hart: state.hart().hart_id },
        None => MemOp::Read,
    };

    let target = mmio_or_l1d(state, engine, paddr, access_size);
    let line_bytes = state.core().l1_d_cache.line_bytes() as u64;
    let width_bytes = ex.op.ctrl.width.bytes();
    let second_line = (!matches!(target, ComponentId::Bus)
        && crosses_cache_line(paddr.val(), width_bytes, line_bytes))
    .then(|| PhysAddr::new((paddr.val() | (line_bytes - 1)) + 1));
    let low_bytes = second_line.map(|second| second.val() - paddr.val());
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;

    let cycle = state.cycle;
    let first_size = low_bytes.map_or(access_size, |low| AccessSize::of_bytes(low as usize));
    let pc = ex.op.pc;
    state.events().schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq { req_id, paddr, vaddr: Some(vaddr), pc: Some(pc), size: first_size, op },
    );
    // The bytes past the line boundary are a second cache access.
    if let (Some(second), Some(low)) = (second_line, low_bytes) {
        let common = engine.common_mut();
        let second_id = common.alloc_req_id();
        let _ = common.memory.load_parts.insert(second_id, req_id);
        state.events().schedule(
            cycle,
            target,
            ComponentId::Pipeline(pipeline_id),
            Packet::MemReq {
                req_id: second_id,
                paddr: second,
                vaddr: Some(vaddr),
                pc: Some(pc),
                size: AccessSize::of_bytes((width_bytes - low) as usize),
                op: MemOp::Read,
            },
        );
    }

    let side_effecting = matches!(target, ComponentId::Bus) || ex.op.ctrl.performs_at_rob_head();
    let parts = low_bytes.map_or(LoadParts::Whole(None), |low| LoadParts::Split {
        low_bytes: low as u8,
        low: None,
        high: None,
    });
    let _ = engine.common_mut().memory.outstanding_loads.insert(
        req_id,
        OutstandingLoad { entry: ex, paddr, vaddr, dirty_updates, side_effecting, parts },
    );
}

/// Trains the load prefetcher on a load by `pc` to `vaddr` (at `paddr`)
/// that has gone to the memory system, and sends the prefetches it wants
/// to the L1D, which fills them or passes them to the L2.
fn send_load_prefetches<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    pc: VirtAddr,
    vaddr: VirtAddr,
    paddr: PhysAddr,
) {
    let prefetches = state.train_load_prefetcher(pc, vaddr, paddr);
    let cycle = state.cycle;
    for prefetch in prefetches {
        let common = engine.common_mut();
        let req_id = common.alloc_req_id();
        let (l1_d_id, pipeline_id) = (common.l1_d_id, common.pipeline_id);
        state.events().schedule(
            cycle,
            ComponentId::Cache(l1_d_id),
            ComponentId::Pipeline(pipeline_id),
            Packet::MemReq {
                req_id,
                paddr: prefetch.paddr,
                vaddr: Some(prefetch.line),
                pc: Some(pc),
                size: AccessSize::Line,
                op: MemOp::Prefetch { into: prefetch.into, exclusive: false },
            },
        );
    }
}

/// Records the parked walk for an access of `access_size` bytes and issues
/// the PTE `MemReq`.
fn park_walk<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    walk_state: crate::uarch::mmu::ptw::WalkState,
    pte_addr: PhysAddr,
    access_size: u64,
    ex: ExMem1Entry,
    translations: PageTranslations,
) {
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let l1_d_id = common.l1_d_id;
    let pipeline_id = common.pipeline_id;
    let _ = common.memory.outstanding_walks.insert(
        req_id,
        OutstandingWalk {
            state: walk_state,
            access_size,
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
            pc: None,
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
    if state.bus.is_ram(paddr, size.bytes() as u64) {
        ComponentId::Cache(engine.common().l1_d_id)
    } else {
        ComponentId::Bus
    }
}

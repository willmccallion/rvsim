//! Fetch1 Stage: PC generation, branch prediction, I-TLB lookup, and
//! instruction-line fetch.
//!
//! Each cycle the stage generates up to `pipeline.width` PCs starting at
//! the current architectural PC, all inside one cache line. For each PC it:
//!
//! 1. Translates the virtual address via `state.translate`. On
//!    [`TranslateOutcome::NeedPte`] it parks the instruction under an
//!    [`OutstandingWalk`] with [`WalkContinuation::Fetch`] and emits a
//!    `MemReq` for the first PTE.
//! 2. Reads the instruction half-word from the RAM fast path so the
//!    branch predictor can examine the encoding inline. Cache-line timing
//!    flows through the packet model — only the data bytes use the
//!    pointer.
//! 3. Asks the branch predictor what the next PC is.
//!
//! The PCs form one [`OutstandingFetch`] group. When the [`FetchBuffer`]
//! already holds the group's line, the group goes straight to the
//! fetch1→fetch2 latch through the program-order reorder buffer. Otherwise
//! fetch1 emits a single line-sized `MemReq` with `op = Fetch` to the L1
//! instruction cache and parks the group under the request id; the
//! mailbox-drain stage releases it when the response arrives. This is
//! gem5's fetch stage: one I-cache access per `fetchBuffer` fill, and none
//! while the PC stays inside the buffered line.

// RISC-V instructions may be misaligned (compressed 16-bit instructions); read_unaligned is intentional.
#![allow(clippy::cast_ptr_alignment)]

use crate::arch::csr::ialign_low_bits;
use crate::common::{AccessType, FetchSeq, InstSeq, LineAddr, PhysAddr, VirtAddr};
use crate::isa::instruction::{InstSize, instruction_size};
use crate::isa::privileged::Trap;
use crate::sim::components::ComponentId;
use crate::sim::packet::{AccessSize, MemOp, Packet};
use crate::trace_branch;
use crate::trace_fetch;
use crate::uarch::bpred::ControlInst;
use crate::uarch::bpred::btb::BranchKind;
use crate::uarch::ctx::StageCtx;
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::pipeline::engine::{BackendCommon, ExecutionEngine};
use crate::uarch::pipeline::frontend::read_inst_half;
use crate::uarch::pipeline::latches::{Fetch1Fetch2Entry, Latch};
use crate::uarch::pipeline::outstanding::{OutstandingFetch, OutstandingWalk, WalkContinuation};

/// The cache line most recently returned by the I-cache (gem5's
/// `fetchBuffer`).
///
/// Timing-only: instruction bytes are always read from the RAM fast path,
/// so the buffer never needs invalidating for correctness. It is emptied
/// when a new line request is issued and refilled when that line's
/// response drains to the fetch1→fetch2 latch. It is a copy of one of the
/// I-cache's lines, so it holds nothing once the cache has been
/// invalidated since it was filled (FENCE.I).
#[derive(Debug, Default)]
pub struct FetchBuffer {
    line: Option<LineAddr>,
    /// The I-cache's invalidation count when the line was filled.
    filled_after: u64,
}

impl FetchBuffer {
    /// True if `line` can be fetched from without an I-cache access, given
    /// the I-cache's current invalidation count.
    #[must_use]
    pub fn holds(&self, line: LineAddr, invalidations: u64) -> bool {
        self.line == Some(line) && self.filled_after == invalidations
    }

    const fn fill(&mut self, line: LineAddr, invalidations: u64) {
        self.line = Some(line);
        self.filled_after = invalidations;
    }

    const fn invalidate(&mut self) {
        self.line = None;
    }
}

/// Accumulates one cycle's fetch entries into an [`OutstandingFetch`].
#[derive(Default)]
struct GroupBuilder {
    fetch_seq: Option<FetchSeq>,
    line: Option<LineAddr>,
    entries: Vec<Fetch1Fetch2Entry>,
}

impl GroupBuilder {
    fn push(
        &mut self,
        common: &mut BackendCommon,
        entry: Fetch1Fetch2Entry,
        line: Option<LineAddr>,
    ) {
        if self.fetch_seq.is_none() {
            self.fetch_seq = Some(common.alloc_fetch_seq());
        }
        if self.line.is_none() {
            self.line = line;
        }
        self.entries.push(entry);
    }

    /// A group that fetches `line` and carries no instruction: the line an
    /// instruction straddling into the next one needs first.
    const fn request_line(&mut self, common: &mut BackendCommon, line: LineAddr) {
        if self.fetch_seq.is_none() {
            self.fetch_seq = Some(common.alloc_fetch_seq());
        }
        self.line = Some(line);
    }

    fn finish(self) -> Option<OutstandingFetch> {
        self.fetch_seq.map(|fetch_seq| OutstandingFetch {
            fetch_seq,
            line: self.line,
            entries: self.entries,
        })
    }
}

/// What fetch predicts for one instruction: where it goes next, whether
/// fetch stops after it, and the kind of control flow for the trace.
struct ControlFlowPrediction {
    target: Option<VirtAddr>,
    stop: bool,
    kind: Option<&'static str>,
}

/// Predicts where fetch goes after the instruction at `pc` from what the
/// BTB records there, as a real front end does before the instruction is
/// decoded: an instruction the BTB misses is fetched past as if it did not
/// branch, and decode finds it. A BTB hit predicts the recorded kind of
/// control instruction; a call pushes its return address at once.
fn predict_control_flow(
    state: &mut StageCtx<'_>,
    seq: InstSeq,
    pc: VirtAddr,
    size: InstSize,
) -> ControlFlowPrediction {
    let predictor = &mut state.core_mut().branch_predictor;
    let Some(hit) = predictor.btb_lookup(pc) else {
        return ControlFlowPrediction { target: None, stop: false, kind: None };
    };
    let control = ControlInst::from_btb(hit, pc.wrapping_add(size.as_u64()));
    let kind = match hit.kind {
        BranchKind::Conditional => "branch",
        BranchKind::Jump { call: true } => "call",
        BranchKind::Jump { call: false } => "jump",
        BranchKind::Indirect { returns: true, .. } => "return",
        BranchKind::Indirect { call: true, .. } => "indirect-call",
        BranchKind::Indirect { .. } => "indirect",
    };
    let target = predictor.predict(seq, pc, control);
    let stop = target.is_some() || !matches!(control, ControlInst::Branch { .. });
    ControlFlowPrediction { target, stop, kind: Some(kind) }
}

/// Holds fetch at `pc` for `cycles`: the translation hit the L2 ITLB, which
/// has now refilled the L1, and the instruction is fetched again after the
/// L2's latency.
fn hold_fetch<E: ExecutionEngine>(state: &StageCtx<'_>, engine: &mut E, pc: VirtAddr, cycles: u64) {
    let common = engine.common_mut();
    common.fetch_hold_until = state.cycle + cycles;
    common.fetch_resume_pc = Some(pc);
}

/// Fetch translates a half-word at a time, the smallest instruction.
const FETCH_TRANSLATION_BYTES: u64 = 2;

/// A latch entry for an instruction that faulted before it could be fetched.
const fn fault_entry(seq: InstSeq, pc: VirtAddr, trap: Trap) -> Fetch1Fetch2Entry {
    Fetch1Fetch2Entry {
        pc,
        paddr: PhysAddr::new(0),
        upper_paddr: None,
        predicted_target: None,
        trap: Some(trap),
        seq,
    }
}

/// Parks an instruction whose translation needs a page-table walk and
/// emits the first PTE read. The group sequence number is reserved now so
/// the instruction drains after everything fetch1 issued before it.
fn park_fetch_walk<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    walk_state: crate::uarch::mmu::ptw::WalkState,
    pte_addr: PhysAddr,
    entry: Fetch1Fetch2Entry,
) {
    let common = engine.common_mut();
    let fetch_seq = common.alloc_fetch_seq();
    let req_id = common.alloc_req_id();
    let l1_d_id = common.l1_d_id;
    let pipeline_id = common.pipeline_id;
    let _ = common.outstanding_walks.insert(
        req_id,
        OutstandingWalk {
            state: walk_state,
            access_size: FETCH_TRANSLATION_BYTES,
            continuation: WalkContinuation::Fetch { fetch_seq, entry },
        },
    );
    common.fetch_walk_pending = true;

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

/// Hands a fetch group to the frontend.
///
/// A group whose line the fetch buffer already holds (or that needs no line
/// at all) enters the program-order reorder buffer immediately; any other
/// group costs one line-sized I-cache request and waits for the response.
pub fn dispatch_fetch_group<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Latch<Fetch1Fetch2Entry>,
    group: OutstandingFetch,
) {
    let invalidations = state.core().l1_i_cache.invalidations();
    match group.line {
        Some(line) if !fetch_buffer.holds(line, invalidations) => {
            issue_line_fetch(state, engine, fetch_buffer, line, group);
        }
        _ => {
            let now = state.cycle;
            let common = engine.common_mut();
            let _ = common.fetch_reorder.insert(group.fetch_seq, group);
            drain_fetch_reorder(now, common, fetch_buffer, latch, invalidations);
        }
    }
}

fn issue_line_fetch<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    fetch_buffer: &mut FetchBuffer,
    line: LineAddr,
    group: OutstandingFetch,
) {
    trace_fetch!(state.config.general.trace_instructions;
        line        = %crate::common::trace::Hex(line.val()),
        fetch_seq   = group.fetch_seq.val(),
        entries     = group.entries.len(),
        "F1: line fetch issued"
    );
    fetch_buffer.invalidate();
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let l1_i_id = common.l1_i_id;
    let pipeline_id = common.pipeline_id;
    let vaddr = group.entries.first().map(|e| e.pc);
    let _ = common.outstanding_fetches.insert(req_id, group);

    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        ComponentId::Cache(l1_i_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: line.phys(),
            vaddr,
            pc: vaddr,
            size: AccessSize::Line,
            op: MemOp::Fetch,
        },
    );
}

/// Releases completed groups into the fetch1→fetch2 latch in program order.
///
/// Stops at the first gap (an older group still waiting on its line). A
/// drained group's line becomes the fetch buffer's content, filled under
/// the I-cache's current invalidation count.
pub fn drain_fetch_reorder(
    now: u64,
    common: &mut BackendCommon,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Latch<Fetch1Fetch2Entry>,
    invalidations: u64,
) {
    loop {
        let next_seq = common.next_emit_fetch_seq;
        let Some(group) = common.fetch_reorder.remove(&next_seq) else {
            break;
        };
        common.next_emit_fetch_seq = next_seq.next();
        if let Some(line) = group.line {
            fetch_buffer.fill(line, invalidations);
        }
        latch.push(now, group.entries);
    }
}

/// Executes the Fetch1 stage: forms up to `pipeline.width` instructions
/// from one cache line into a fetch group, advancing `fetch_pc` to the
/// predicted next PC.
///
/// The caller runs this only while no fetch is in flight
/// ([`BackendCommon::fetch_in_flight`]), so a parked fetch walk or an
/// unanswered line request never gets a duplicate request for the same PC.
pub fn fetch1_stage<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Latch<Fetch1Fetch2Entry>,
    fetch_pc: &mut VirtAddr,
) {
    let pc = engine.common_mut().fetch_resume_pc.take().unwrap_or(*fetch_pc);
    let line_bytes = state.core().l1_i_cache.line_bytes() as u64;
    let mut cursor = FetchCursor {
        pc,
        line_bytes,
        line_end: (pc.val() | (line_bytes - 1)) + 1,
        group: GroupBuilder::default(),
    };
    for _ in 0..state.config.pipeline.fetch_width() {
        if matches!(fetch_instruction(state, engine, fetch_buffer, &mut cursor), FetchStep::Stop) {
            break;
        }
    }
    if let Some(group) = cursor.group.finish() {
        dispatch_fetch_group(state, engine, fetch_buffer, latch, group);
    }
    *fetch_pc = cursor.pc;
}

/// One cycle's fetch so far: the next PC, the end of the cache line it
/// fetches from, and the group of instructions fetched.
struct FetchCursor {
    pc: VirtAddr,
    line_bytes: u64,
    line_end: u64,
    group: GroupBuilder,
}

/// Whether fetch goes on to another instruction this cycle.
enum FetchStep {
    Next,
    Stop,
}

/// What translating one half-word of an instruction for fetch gave.
enum FetchHalf {
    Ready(PhysAddr),
    Fault(Trap),
    /// Fetch stops at it: the L2 ITLB answered, or a page-table walk is
    /// parked. Either way the encoding, and so the next PC, is unknown.
    Stalled,
}

/// Fetches the instruction at the cursor's PC into its group.
fn fetch_instruction<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    fetch_buffer: &FetchBuffer,
    cursor: &mut FetchCursor,
) -> FetchStep {
    let pc = cursor.pc;
    if pc.val() + FETCH_TRANSLATION_BYTES > cursor.line_end {
        return FetchStep::Stop;
    }
    let Some(paddr) = fetch_lower_half(state, engine, &mut cursor.group, pc) else {
        return FetchStep::Stop;
    };
    let half_word = read_inst_half(state, paddr.val());
    let size = instruction_size(half_word, state.hart().csrs.misa);
    let is_compressed = matches!(size, InstSize::Compressed(_));
    // A 32-bit instruction whose upper half lies in the next line
    // completes only when that line arrives: it heads a group that
    // fetches the next line, once this line is in the fetch buffer.
    let straddles_line = !is_compressed && pc.wrapping_add(4).val() > cursor.line_end;
    if straddles_line {
        let line = LineAddr::from_phys(paddr, cursor.line_bytes);
        if !cursor.group.entries.is_empty() {
            return FetchStep::Stop;
        }
        if !fetch_buffer.holds(line, state.core().l1_i_cache.invalidations()) {
            cursor.group.request_line(engine.common_mut(), line);
            return FetchStep::Stop;
        }
        cursor.line_end += cursor.line_bytes;
    }
    let seq = engine.common_mut().alloc_inst_seq();
    state.trace_fetched(seq, pc);
    let lower = Fetch1Fetch2Entry { pc, paddr, seq, ..Fetch1Fetch2Entry::default() };
    let (entry, bits, line_paddr) = if is_compressed {
        (lower, u32::from(half_word), paddr)
    } else {
        let Some(entry) = fetch_upper_half(state, engine, &mut cursor.group, lower) else {
            return FetchStep::Stop;
        };
        let upper = entry
            .upper_paddr
            .unwrap_or_else(|| PhysAddr::new(paddr.val() + FETCH_TRANSLATION_BYTES));
        let upper_half = read_inst_half(state, upper.val());
        let line_paddr = if straddles_line { upper } else { paddr };
        (entry, (u32::from(upper_half) << 16) | u32::from(half_word), line_paddr)
    };
    let line = LineAddr::from_phys(line_paddr, cursor.line_bytes);
    push_fetched(state, engine, cursor, FetchedInst { entry, bits, size, line })
}

/// The physical address of the instruction at `pc`, or `None` when fetch
/// stops at it: a misaligned PC or a translation fault ends the group with
/// a fault entry.
fn fetch_lower_half<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    group: &mut GroupBuilder,
    pc: VirtAddr,
) -> Option<PhysAddr> {
    let is_misaligned = pc.val() & ialign_low_bits(state.hart().csrs.misa) != 0;
    let lower = if is_misaligned {
        FetchHalf::Fault(Trap::InstructionAddressMisaligned(pc.val()))
    } else {
        // Not reported to the trace yet on a walk: the instruction is
        // fetched again, under a new number, once the walk completes.
        translate_fetch_half(state, engine, pc, pc, |common| Fetch1Fetch2Entry {
            pc,
            seq: common.alloc_inst_seq(),
            ..Fetch1Fetch2Entry::default()
        })
    };
    match lower {
        FetchHalf::Ready(paddr) => Some(paddr),
        FetchHalf::Fault(trap) => {
            trace_fetch!(state.config.general.trace_instructions;
                pc          = %crate::common::trace::Hex(pc.val()),
                trap        = ?trap,
                "F1: fetch trap"
            );
            let seq = engine.common_mut().alloc_inst_seq();
            state.trace_fetched(seq, pc);
            group.push(engine.common_mut(), fault_entry(seq, pc, trap), None);
            None
        }
        FetchHalf::Stalled => None,
    }
}

/// `lower`, a 32-bit instruction's entry, with its upper half-word's
/// physical address when that lies on the next page; `None` when fetch
/// stops at it, a translation fault ending the group with a fault entry.
fn fetch_upper_half<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    group: &mut GroupBuilder,
    lower: Fetch1Fetch2Entry,
) -> Option<Fetch1Fetch2Entry> {
    let upper_va = lower.pc.wrapping_add(FETCH_TRANSLATION_BYTES);
    if lower.pc.page_number() == upper_va.page_number() {
        return Some(lower);
    }
    let pending = lower.clone();
    match translate_fetch_half(state, engine, lower.pc, upper_va, |_| pending) {
        FetchHalf::Ready(upper) => Some(Fetch1Fetch2Entry { upper_paddr: Some(upper), ..lower }),
        FetchHalf::Fault(trap) => {
            trace_fetch!(state.config.general.trace_instructions;
                pc           = %crate::common::trace::Hex(lower.pc.val()),
                paddr        = %crate::common::trace::Hex(lower.paddr.val()),
                trap         = ?trap,
                crosses_page = true,
                "F1: fetch trap on the upper half-word"
            );
            group.push(engine.common_mut(), fault_entry(lower.seq, lower.pc, trap), None);
            None
        }
        FetchHalf::Stalled => None,
    }
}

/// Translates the half-word at `va` of the instruction at `pc` for fetch.
/// An L2 ITLB hit holds fetch at `pc`; a page-table walk parks the entry
/// `pending` builds until the walk completes and fetch tries `pc` again.
fn translate_fetch_half<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    pc: VirtAddr,
    va: VirtAddr,
    pending: impl FnOnce(&mut BackendCommon) -> Fetch1Fetch2Entry,
) -> FetchHalf {
    match state.translate(va, AccessType::Fetch, FETCH_TRANSLATION_BYTES) {
        TranslateOutcome::Ready(r) if r.cycles > 0 && r.trap.is_none() => {
            hold_fetch(state, engine, pc, r.cycles);
            FetchHalf::Stalled
        }
        TranslateOutcome::Ready(r) => r.trap.map_or(FetchHalf::Ready(r.paddr), FetchHalf::Fault),
        TranslateOutcome::NeedPte { pte_addr, state: walk_state } => {
            let entry = pending(engine.common_mut());
            park_fetch_walk(state, engine, walk_state, pte_addr, entry);
            FetchHalf::Stalled
        }
    }
}

/// An instruction fetch read: its latch entry, encoding, size and the
/// cache line it is fetched from.
struct FetchedInst {
    entry: Fetch1Fetch2Entry,
    bits: u32,
    size: InstSize,
    line: LineAddr,
}

/// Adds `fetched` to the cursor's group with its predicted next PC, and
/// moves the cursor there.
fn push_fetched<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    cursor: &mut FetchCursor,
    fetched: FetchedInst,
) -> FetchStep {
    let FetchedInst { entry, bits, size, line } = fetched;
    let prediction = predict_control_flow(state, entry.seq, entry.pc, size);
    let next_pc = prediction.target.unwrap_or_else(|| entry.pc.wrapping_add(size.as_u64()));
    if let Some(kind) = prediction.kind {
        trace_branch!(state.config.general.trace_instructions;
            event       = "predict",
            pc          = %crate::common::trace::Hex(entry.pc.val()),
            paddr       = %crate::common::trace::Hex(entry.paddr.val()),
            inst        = %crate::common::trace::Hex32(bits),
            bp_type     = kind,
            pred_taken  = prediction.target.is_some(),
            next_pc     = %crate::common::trace::Hex(next_pc.val()),
            "F1: control-flow prediction"
        );
    }
    trace_fetch!(state.config.general.trace_instructions;
        pc          = %crate::common::trace::Hex(entry.pc.val()),
        paddr       = %crate::common::trace::Hex(entry.paddr.val()),
        compressed  = matches!(size, InstSize::Compressed(_)),
        pred_taken  = prediction.target.is_some(),
        next_pc     = %crate::common::trace::Hex(next_pc.val()),
        "F1: fetch entry issued"
    );
    let entry = Fetch1Fetch2Entry { predicted_target: prediction.target, ..entry };
    cursor.group.push(engine.common_mut(), entry, Some(line));
    cursor.pc = next_pc;
    if prediction.stop { FetchStep::Stop } else { FetchStep::Next }
}

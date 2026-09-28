//! Fetch1 Stage: PC generation, branch prediction, I-TLB lookup, and
//! instruction-line fetch.
//!
//! Each cycle the stage generates up to `pipeline.width` PCs starting at
//! the current architectural PC, all inside one cache line. For each PC it:
//!
//! 1. Translates the virtual address via `state.translate`. On
//!    [`TranslateResult::NeedPte`] it parks the instruction under an
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

use crate::common::constants::{COMPRESSED_INSTRUCTION_MASK, COMPRESSED_INSTRUCTION_VALUE};
use crate::common::{AccessType, ExceptionStage, LineAddr, PhysAddr, Trap, VirtAddr};
use crate::common::{InstSeq, InstSize};
use crate::core::arch::csr;
use crate::core::pipeline::engine::{BackendCommon, ExecutionEngine};
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, Latch};
use crate::core::pipeline::outstanding::{OutstandingFetch, OutstandingWalk, WalkContinuation};
use crate::core::units::bru::ControlInst;
use crate::core::units::bru::btb::BranchKind;
use crate::isa::rvc::expand::expand;
use crate::sim::StageCtx;
use crate::sim::components::ComponentId;
use crate::sim::packet::{AccessSize, MemOp, Packet};
use crate::sim::state::memory::TranslateResult;
use crate::trace_branch;
use crate::trace_fetch;

/// The cache line most recently returned by the I-cache (gem5's
/// `fetchBuffer`).
///
/// Timing-only: instruction bytes are always read from the RAM fast path,
/// so the buffer never needs invalidating for correctness. It is emptied
/// when a new line request is issued and refilled when that line's
/// response drains to the fetch1→fetch2 latch.
#[derive(Debug, Default)]
pub struct FetchBuffer {
    line: Option<LineAddr>,
}

impl FetchBuffer {
    /// True if `line` can be fetched from without an I-cache access.
    #[must_use]
    pub fn holds(&self, line: LineAddr) -> bool {
        self.line == Some(line)
    }

    const fn fill(&mut self, line: LineAddr) {
        self.line = Some(line);
    }

    const fn invalidate(&mut self) {
        self.line = None;
    }
}

/// Accumulates one cycle's fetch entries into an [`OutstandingFetch`].
#[derive(Default)]
struct GroupBuilder {
    fetch_seq: Option<u64>,
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

/// Reads a 16-bit instruction half-word from the RAM fast-path pointer.
///
/// Returns 0 for addresses outside DRAM. Architecturally, fetching from
/// MMIO returns garbage; the decoded `0` results in an illegal-instruction
/// trap, which matches what real hardware would do.
fn read_inst_half(state: &StageCtx<'_>, paddr: u64) -> u16 {
    state.bus.ram_region().filter(|r| r.contains(paddr, 2)).map_or(0u16, |r| {
        // SAFETY: `RamRegion::contains(paddr, 2)` bounds-checks the access.
        unsafe { r.ptr(paddr).cast::<u16>().read_unaligned() }
    })
}

/// What fetch predicts for one instruction: where it goes next, whether
/// fetch stops after it, and the kind of control flow for the trace.
struct ControlFlowPrediction {
    target: Option<u64>,
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
    pc: u64,
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
fn hold_fetch<E: ExecutionEngine>(state: &StageCtx<'_>, engine: &mut E, pc: u64, cycles: u64) {
    let common = engine.common_mut();
    common.fetch_hold_until = state.cycle + cycles;
    common.fetch_resume_pc = Some(pc);
}

/// A latch entry for an instruction that faulted before it could be fetched.
const fn fault_entry(seq: InstSeq, pc: u64, trap: Trap) -> Fetch1Fetch2Entry {
    Fetch1Fetch2Entry {
        pc,
        paddr: PhysAddr::new(0),
        upper_paddr: None,
        pred_taken: false,
        pred_target: 0,
        trap: Some(trap),
        exception_stage: Some(ExceptionStage::Fetch),
        seq,
    }
}

/// Parks an instruction whose translation needs a page-table walk and
/// emits the first PTE read. The group sequence number is reserved now so
/// the instruction drains after everything fetch1 issued before it.
fn park_fetch_walk<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    engine: &mut E,
    walk_state: crate::core::units::mmu::ptw::WalkState,
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
            pte_addr,
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
    match group.line {
        Some(line) if !fetch_buffer.holds(line) => {
            issue_line_fetch(state, engine, fetch_buffer, line, group);
        }
        _ => {
            let now = state.cycle;
            let common = engine.common_mut();
            let _ = common.fetch_reorder.insert(group.fetch_seq, group);
            drain_fetch_reorder(now, common, fetch_buffer, latch);
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
        line        = %crate::trace::Hex(line.val()),
        fetch_seq   = group.fetch_seq,
        entries     = group.entries.len(),
        "F1: line fetch issued"
    );
    fetch_buffer.invalidate();
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let l1_i_id = common.l1_i_id;
    let pipeline_id = common.pipeline_id;
    let vaddr = group.entries.first().map(|e| VirtAddr::new(e.pc));
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
            size: AccessSize::Line,
            op: MemOp::Fetch,
        },
    );
}

/// Releases completed groups into the fetch1→fetch2 latch in program order.
///
/// Stops at the first gap (an older group still waiting on its line). A
/// drained group's line becomes the fetch buffer's content.
pub fn drain_fetch_reorder(
    now: u64,
    common: &mut BackendCommon,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Latch<Fetch1Fetch2Entry>,
) {
    loop {
        let next_seq = common.next_emit_fetch_seq;
        let Some(group) = common.fetch_reorder.remove(&next_seq) else {
            break;
        };
        common.next_emit_fetch_seq = next_seq.wrapping_add(1);
        if let Some(line) = group.line {
            fetch_buffer.fill(line);
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
    fetch_pc: &mut u64,
) {
    let mut current_pc = engine.common_mut().fetch_resume_pc.take().unwrap_or(*fetch_pc);
    let align_mask = csr::ialign_low_bits(state.hart().csrs.misa);

    let line_bytes = state.core().l1_i_cache.line_bytes() as u64;
    let mut line_end = (current_pc | (line_bytes - 1)) + 1;
    let mut group = GroupBuilder::default();

    for _ in 0..state.config.pipeline.fetch_width() {
        if current_pc + 2 > line_end {
            break;
        }

        let fetch_trap = if (current_pc & align_mask) != 0 {
            Some(Trap::InstructionAddressMisaligned(current_pc))
        } else {
            None
        };

        // 1. Translate the PC.
        let translated = if fetch_trap.is_none() {
            state.translate(VirtAddr::new(current_pc), AccessType::Fetch, 2)
        } else {
            TranslateResult::Ready(crate::common::TranslationResult::success(PhysAddr::new(0), 0))
        };

        let (paddr, trap) = match translated {
            TranslateResult::Ready(r) if r.cycles > 0 && r.trap.is_none() => {
                hold_fetch(state, engine, current_pc, r.cycles);
                break;
            }
            TranslateResult::Ready(r) => (r.paddr, r.trap),
            TranslateResult::NeedPte { pte_addr, state: walk_state } => {
                let pending = Fetch1Fetch2Entry {
                    pc: current_pc,
                    paddr: PhysAddr::new(0),
                    upper_paddr: None,
                    pred_taken: false,
                    pred_target: 0,
                    trap: None,
                    exception_stage: None,
                    seq: engine.common_mut().alloc_inst_seq(),
                };
                park_fetch_walk(state, engine, walk_state, pte_addr, pending);
                // Fetch holds here until the translation returns: the
                // encoding, and so the next PC, is unknown until then.
                break;
            }
        };

        if let Some(trap_cause) = fetch_trap.or(trap) {
            trace_fetch!(state.config.general.trace_instructions;
                pc          = %crate::trace::Hex(current_pc),
                trap        = ?trap_cause,
                "F1: fetch trap"
            );
            let seq = engine.common_mut().alloc_inst_seq();
            group.push(engine.common_mut(), fault_entry(seq, current_pc, trap_cause), None);
            break;
        }

        let phys_addr = paddr.val();
        let mut line = LineAddr::from_phys(paddr, line_bytes);
        let half_word = read_inst_half(state, phys_addr);
        let is_compressed =
            (half_word & COMPRESSED_INSTRUCTION_MASK) != COMPRESSED_INSTRUCTION_VALUE;
        let step = if is_compressed { InstSize::Compressed } else { InstSize::Standard };

        // A 32-bit instruction whose upper half lies in the next line
        // completes only when that line arrives: it heads a group that
        // fetches the next line, once this line is in the fetch buffer.
        let straddles_line = !is_compressed && current_pc.wrapping_add(4) > line_end;
        if straddles_line {
            if !group.entries.is_empty() {
                break;
            }
            if !fetch_buffer.holds(line) {
                group.request_line(engine.common_mut(), line);
                break;
            }
            line_end += line_bytes;
        }

        let mut next_pc_calc = current_pc.wrapping_add(step.as_u64());
        let mut pred_taken = false;
        let mut pred_target = 0;
        let mut upper_paddr = None;
        let seq = engine.common_mut().alloc_inst_seq();

        let full_inst = if is_compressed {
            expand(half_word)
        } else {
            let upper_va = current_pc.wrapping_add(2);
            let crosses_page = (current_pc >> 12) != (upper_va >> 12);
            let upper_phys = if crosses_page {
                match state.translate(VirtAddr::new(upper_va), AccessType::Fetch, 2) {
                    TranslateResult::Ready(r) if r.cycles > 0 && r.trap.is_none() => {
                        hold_fetch(state, engine, current_pc, r.cycles);
                        break;
                    }
                    TranslateResult::Ready(r) => {
                        if let Some(trap) = r.trap {
                            trace_fetch!(state.config.general.trace_instructions;
                                pc           = %crate::trace::Hex(current_pc),
                                paddr        = %crate::trace::Hex(phys_addr),
                                trap         = ?trap,
                                crosses_page = true,
                                "F1: fetch trap on the upper half-word"
                            );
                            group.push(
                                engine.common_mut(),
                                fault_entry(seq, current_pc, trap),
                                None,
                            );
                            break;
                        }
                        r.paddr
                    }
                    TranslateResult::NeedPte { pte_addr, state: walk_state } => {
                        let pending = Fetch1Fetch2Entry {
                            pc: current_pc,
                            paddr,
                            upper_paddr: None,
                            pred_taken: false,
                            pred_target: 0,
                            trap: None,
                            exception_stage: None,
                            seq,
                        };
                        park_fetch_walk(state, engine, walk_state, pte_addr, pending);
                        // As for the lower half: fetch holds until the
                        // translation returns, then fetches this again.
                        break;
                    }
                }
            } else {
                PhysAddr::new(phys_addr + 2)
            };
            upper_paddr = crosses_page.then_some(upper_phys);
            if straddles_line {
                line = LineAddr::from_phys(upper_phys, line_bytes);
            }

            let upper_raw = upper_phys.val();
            let upper_half = read_inst_half(state, upper_raw);
            (upper_half as u32) << 16 | (half_word as u32)
        };

        let prediction = predict_control_flow(state, seq, current_pc, step);
        if let Some(target) = prediction.target {
            next_pc_calc = target;
            pred_taken = true;
            pred_target = target;
        }
        let stop_fetch = prediction.stop;
        if let Some(kind) = prediction.kind {
            trace_branch!(state.config.general.trace_instructions;
                event       = "predict",
                pc          = %crate::trace::Hex(current_pc),
                paddr       = %crate::trace::Hex(phys_addr),
                inst        = %crate::trace::Hex32(full_inst),
                bp_type     = kind,
                pred_taken,
                pred_target = %crate::trace::Hex(pred_target),
                "F1: control-flow prediction"
            );
        }

        trace_fetch!(state.config.general.trace_instructions;
            pc          = %crate::trace::Hex(current_pc),
            paddr       = %crate::trace::Hex(phys_addr),
            compressed  = is_compressed,
            pred_taken,
            pred_target = %crate::trace::Hex(pred_target),
            "F1: fetch entry issued"
        );

        let entry = Fetch1Fetch2Entry {
            pc: current_pc,
            paddr,
            upper_paddr,
            pred_taken,
            pred_target,
            trap: None,
            exception_stage: None,
            seq,
        };
        group.push(engine.common_mut(), entry, Some(line));

        current_pc = next_pc_calc;
        if stop_fetch {
            break;
        }
    }

    if let Some(group) = group.finish() {
        dispatch_fetch_group(state, engine, fetch_buffer, latch, group);
    }
    *fetch_pc = current_pc;
}

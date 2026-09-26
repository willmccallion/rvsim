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

use crate::common::InstSize;
use crate::common::constants::{
    COMPRESSED_INSTRUCTION_MASK, COMPRESSED_INSTRUCTION_VALUE, OPCODE_MASK, RD_MASK, RD_SHIFT,
    RS1_MASK, RS1_SHIFT,
};
use crate::common::{AccessType, ExceptionStage, LineAddr, PhysAddr, RegIdx, Trap, VirtAddr};
use crate::core::arch::csr;
use crate::core::pipeline::engine::{BackendCommon, ExecutionEngine};
use crate::core::pipeline::latches::Fetch1Fetch2Entry;
use crate::core::pipeline::outstanding::{OutstandingFetch, OutstandingWalk, WalkContinuation};
use crate::core::units::bru::{BranchPredictor, Ghr, RasSnapshot};
use crate::isa::abi;
use crate::isa::decode::{decode_b_type_imm, decode_j_type_imm};
use crate::isa::rv64i::opcodes;
use crate::isa::rvc::expand::expand;
use crate::sim::CoreCtx;
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

/// Which half of a 32-bit instruction an outstanding fetch walk translates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchWalkHalf {
    /// The instruction's own PC; `paddr` is unknown until the walk completes.
    Lower,
    /// The upper half-word of an instruction straddling a page boundary;
    /// `paddr` (the lower half) is already known and stays as is.
    Upper,
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
fn read_inst_half(state: &CoreCtx<'_>, paddr: u64) -> u16 {
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

/// Predicts an instruction's control flow from its encoding, the way a
/// predecoded fetch line lets a real front end: a direct branch or jump
/// takes its target from the immediate, a return pops the RAS, an indirect
/// jump asks the BTB, and a call pushes its return address at once.
fn predict_control_flow(
    state: &mut CoreCtx<'_>,
    pc: u64,
    size: InstSize,
    inst: u32,
) -> ControlFlowPrediction {
    let bp = &mut state.core.branch_predictor;
    let opcode = inst & OPCODE_MASK;
    let rd = RegIdx::new(((inst >> RD_SHIFT) & RD_MASK) as u8);
    let rs1 = RegIdx::new(((inst >> RS1_SHIFT) & RS1_MASK) as u8);
    let rd_link = rd == abi::REG_RA || rd == abi::REG_T0;
    let rs1_link = rs1 == abi::REG_RA || rs1 == abi::REG_T0;
    let return_address = pc.wrapping_add(size.as_u64());
    match opcode {
        opcodes::OP_BRANCH => {
            let (taken, btb_target) = bp.predict_branch(pc);
            bp.speculate(pc, taken);
            let target = taken.then(|| {
                btb_target.unwrap_or_else(|| pc.wrapping_add(decode_b_type_imm(inst) as u64))
            });
            ControlFlowPrediction { target, stop: taken, kind: Some("branch") }
        }
        opcodes::OP_JAL => {
            if rd_link {
                bp.push_return(return_address);
            }
            ControlFlowPrediction {
                target: Some(pc.wrapping_add(decode_j_type_imm(inst) as u64)),
                stop: true,
                kind: Some(if rd_link { "call" } else { "jump" }),
            }
        }
        opcodes::OP_JALR => {
            let is_return = rs1_link && (!rd_link || rd != rs1);
            let target = if is_return { bp.pop_return() } else { bp.predict_btb(pc) };
            if rd_link {
                bp.push_return(return_address);
            }
            let kind = if is_return {
                "return"
            } else if rd_link {
                "indirect-call"
            } else {
                "indirect"
            };
            ControlFlowPrediction { target, stop: true, kind: Some(kind) }
        }
        _ => ControlFlowPrediction { target: None, stop: false, kind: None },
    }
}

/// Holds fetch at `pc` for `cycles`: the translation hit the L2 ITLB, which
/// has now refilled the L1, and the instruction is fetched again after the
/// L2's latency.
fn hold_fetch<E: ExecutionEngine>(state: &CoreCtx<'_>, engine: &mut E, pc: u64, cycles: u64) {
    let common = engine.common_mut();
    common.fetch_hold_until = state.cycle + cycles;
    common.fetch_resume_pc = Some(pc);
}

/// A latch entry for an instruction that faulted before it could be fetched.
fn fault_entry(pc: u64, trap: Trap) -> Fetch1Fetch2Entry {
    Fetch1Fetch2Entry {
        pc,
        paddr: PhysAddr::new(0),
        upper_paddr: None,
        pred_taken: false,
        pred_target: 0,
        trap: Some(trap),
        exception_stage: Some(ExceptionStage::Fetch),
        ghr_snapshot: Ghr::default(),
        ras_snapshot: RasSnapshot::default(),
    }
}

/// Parks an instruction whose translation needs a page-table walk and
/// emits the first PTE read. The group sequence number is reserved now so
/// the instruction drains after everything fetch1 issued before it.
fn park_fetch_walk<E: ExecutionEngine>(
    state: &mut CoreCtx<'_>,
    engine: &mut E,
    walk_state: crate::core::units::mmu::ptw::WalkState,
    pte_addr: PhysAddr,
    entry: Fetch1Fetch2Entry,
    half: FetchWalkHalf,
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
            continuation: WalkContinuation::Fetch { fetch_seq, entry, half },
        },
    );
    common.fetch_walk_pending = true;

    let cycle = state.cycle;
    state.event_queue.schedule(
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
    state: &mut CoreCtx<'_>,
    engine: &mut E,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Vec<Fetch1Fetch2Entry>,
    group: OutstandingFetch,
) {
    match group.line {
        Some(line) if !fetch_buffer.holds(line) => {
            issue_line_fetch(state, engine, fetch_buffer, line, group);
        }
        _ => {
            let common = engine.common_mut();
            let _ = common.fetch_reorder.insert(group.fetch_seq, group);
            drain_fetch_reorder(common, fetch_buffer, latch);
        }
    }
}

fn issue_line_fetch<E: ExecutionEngine>(
    state: &mut CoreCtx<'_>,
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
    state.event_queue.schedule(
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
    common: &mut BackendCommon,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Vec<Fetch1Fetch2Entry>,
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
        latch.extend(group.entries);
    }
}

/// Executes the Fetch1 stage: forms up to `pipeline.width` instructions
/// from one cache line into a fetch group, advancing the architectural PC
/// by the predicted next-PC.
///
/// The caller runs this only while no fetch is in flight
/// ([`BackendCommon::fetch_in_flight`]), so a parked fetch walk or an
/// unanswered line request never gets a duplicate request for the same PC.
pub fn fetch1_stage<E: ExecutionEngine>(
    state: &mut CoreCtx<'_>,
    engine: &mut E,
    fetch_buffer: &mut FetchBuffer,
    latch: &mut Vec<Fetch1Fetch2Entry>,
) {
    let mut current_pc = engine.common_mut().fetch_resume_pc.take().unwrap_or(state.hart.pc);
    let c_enabled = (state.hart.csrs.misa & csr::MISA_EXT_C) != 0;
    let align_mask: u64 = if c_enabled { 1 } else { 3 };

    let line_bytes = state.core.l1_i_cache.line_bytes() as u64;
    let line_end = (current_pc | (line_bytes - 1)) + 1;
    let mut group = GroupBuilder::default();

    for _ in 0..state.config.pipeline.width {
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
                    ghr_snapshot: Ghr::default(),
                    ras_snapshot: RasSnapshot::default(),
                };
                park_fetch_walk(state, engine, walk_state, pte_addr, pending, FetchWalkHalf::Lower);
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
            group.push(engine.common_mut(), fault_entry(current_pc, trap_cause), None);
            break;
        }

        let phys_addr = paddr.val();
        let line = LineAddr::from_phys(paddr, line_bytes);
        let half_word = read_inst_half(state, phys_addr);
        let is_compressed =
            (half_word & COMPRESSED_INSTRUCTION_MASK) != COMPRESSED_INSTRUCTION_VALUE;
        let step = if is_compressed { InstSize::Compressed } else { InstSize::Standard };

        let mut next_pc_calc = current_pc.wrapping_add(step.as_u64());
        let mut pred_taken = false;
        let mut pred_target = 0;
        let mut upper_paddr = None;
        let ghr_snapshot = state.core.branch_predictor.snapshot_history();
        let ras_snapshot = state.core.branch_predictor.snapshot_ras();

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
                            group.push(engine.common_mut(), fault_entry(current_pc, trap), None);
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
                            ghr_snapshot,
                            ras_snapshot,
                        };
                        park_fetch_walk(
                            state,
                            engine,
                            walk_state,
                            pte_addr,
                            pending,
                            FetchWalkHalf::Upper,
                        );
                        // See the lower-half NeedPte arm above. The
                        // instruction is known to be 32-bit here (compressed
                        // instructions never cross a page).
                        current_pc = current_pc.wrapping_add(4);
                        break;
                    }
                }
            } else {
                PhysAddr::new(phys_addr + 2)
            };
            upper_paddr = crosses_page.then_some(upper_phys);

            let upper_raw = upper_phys.val();
            let upper_half = read_inst_half(state, upper_raw);
            (upper_half as u32) << 16 | (half_word as u32)
        };

        let prediction = predict_control_flow(state, current_pc, step, full_inst);
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
            ghr_snapshot,
            ras_snapshot,
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
    state.hart.pc = current_pc;
}

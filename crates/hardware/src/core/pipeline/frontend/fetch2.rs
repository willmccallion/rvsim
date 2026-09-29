//! Fetch2 Stage: instruction-byte read and compressed-instruction expansion.
//!
//! Consumes entries from the Fetch1→Fetch2 latch (populated either by
//! Fetch1 directly — for trap-bearing entries — or by the mailbox-drain
//! stage when a fetch `MemResp` arrives). For each entry Fetch2:
//!
//! - Reads the half-word from the RAM fast-path pointer (instructions
//!   always reside in DRAM; MMIO fetches return 0, which the decoder
//!   surfaces as an illegal-instruction trap).
//! - If the instruction is compressed, runs RVC expansion.
//! - If the instruction is 32-bit, reads its upper half-word at the
//!   physical address fetch1 translated (its own page, or the next page
//!   when the instruction straddles one).

// RISC-V instructions may be misaligned (compressed 16-bit instructions); read_unaligned is intentional.
#![allow(clippy::cast_ptr_alignment)]

use crate::common::{ExceptionStage, PhysAddr};
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, IfIdEntry};
use crate::isa::instruction::{InstSize, is_compressed};
use crate::isa::privileged::Trap;
use crate::isa::rvc::expand;
use crate::sim::StageCtx;
use crate::{trace_fetch, trace_trap};

/// Reads a 16-bit instruction half-word from the RAM fast-path pointer.
///
/// Returns 0 for addresses outside DRAM (an illegal-instruction trap will
/// surface during decode).
fn read_inst_half(state: &StageCtx<'_>, paddr: u64) -> u16 {
    state.bus.ram_region().filter(|r| r.contains(paddr, 2)).map_or(0u16, |r| {
        // SAFETY: `RamRegion::contains(paddr, 2)` bounds-checks the access.
        unsafe { r.ptr(paddr).cast::<u16>().read_unaligned() }
    })
}

/// Executes the Fetch2 stage: decode each F1→F2 entry into an `IfIdEntry`.
pub fn fetch2_stage(
    state: &mut StageCtx<'_>,
    input: &mut Vec<Fetch1Fetch2Entry>,
    output: &mut Vec<IfIdEntry>,
) {
    output.clear();
    if input.is_empty() {
        return;
    }
    let entries = std::mem::take(input);

    for f1 in entries {
        if let Some(ref trap) = f1.trap {
            trace_trap!(state.trace_trap_enabled(trap);
                event = "propagate",
                stage = "F2",
                pc    = %crate::trace::Hex(f1.pc),
                trap  = ?trap,
                "F2: trap propagated from F1"
            );
            output.push(IfIdEntry {
                pc: f1.pc,
                inst: 0,
                inst_size: InstSize::Standard,
                pred_taken: f1.pred_taken,
                pred_target: f1.pred_target,
                trap: f1.trap,
                exception_stage: f1.exception_stage,
                seq: f1.seq,
            });
            break;
        }

        let phys_addr = f1.paddr.val();
        let half_word = read_inst_half(state, phys_addr);
        let is_compressed = is_compressed(half_word);

        let (inst, step, inst_trap) = if is_compressed {
            let expanded = expand(half_word);
            if expanded == 0 {
                (0, InstSize::Compressed, Some(Trap::IllegalInstruction(half_word as u32)))
            } else {
                (expanded, InstSize::Compressed, None)
            }
        } else {
            let upper_paddr = f1.upper_paddr.unwrap_or_else(|| PhysAddr::new(phys_addr + 2));
            let upper_half = read_inst_half(state, upper_paddr.val());
            let full_inst = (upper_half as u32) << 16 | (half_word as u32);
            (full_inst, InstSize::Standard, None)
        };

        if let Some(t) = inst_trap {
            trace_trap!(state.trace_trap_enabled(&t);
                event = "decode-trap",
                stage = "F2",
                pc    = %crate::trace::Hex(f1.pc),
                trap  = ?t,
                "F2: instruction decode trap"
            );
            output.push(IfIdEntry {
                pc: f1.pc,
                inst: 0,
                inst_size: step,
                pred_taken: f1.pred_taken,
                pred_target: f1.pred_target,
                trap: Some(t),
                exception_stage: Some(ExceptionStage::Fetch),
                seq: f1.seq,
            });
            break;
        }

        trace_fetch!(state.config.general.trace_instructions;
            pc         = %crate::trace::Hex(f1.pc),
            inst       = inst,
            inst_size  = step.as_u64(),
            compressed = is_compressed,
            "F2: decoded instruction"
        );

        output.push(IfIdEntry {
            pc: f1.pc,
            inst,
            inst_size: step,
            pred_taken: f1.pred_taken,
            pred_target: f1.pred_target,
            trap: None,
            exception_stage: None,
            seq: f1.seq,
        });
    }
}

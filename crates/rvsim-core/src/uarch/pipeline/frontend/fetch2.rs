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

use crate::common::PhysAddr;
use crate::common::trace::Hex;
use crate::isa::instruction::{InstSize, instruction_size};
use crate::isa::privileged::Trap;
use crate::isa::rvc::expand;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::frontend::read_inst_half;
use crate::uarch::pipeline::latches::{Fetch1Fetch2Entry, IfIdEntry};
use crate::{trace_fetch, trace_trap};

/// Executes the Fetch2 stage: decode each F1→F2 entry into an `IfIdEntry`.
pub fn fetch2_stage(
    state: &StageCtx<'_>,
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
            trace_trap!(state.uncore().trace_trap_enabled(trap);
                event = "propagate",
                stage = "F2",
                pc    = %Hex(f1.pc.val()),
                trap  = ?trap,
                "F2: trap propagated from F1"
            );
            output.push(IfIdEntry {
                pc: f1.pc,
                bits: 0,
                inst_size: InstSize::Standard,
                predicted_target: f1.predicted_target,
                trap: f1.trap,
                seq: f1.seq,
            });
            break;
        }

        let phys_addr = f1.paddr.val();
        let half_word = read_inst_half(state, phys_addr);
        let size = instruction_size(half_word, state.hart().csrs.misa);
        let is_compressed = matches!(size, InstSize::Compressed(_));

        let (inst, step, inst_trap) = if is_compressed {
            expand(half_word).map_or_else(
                || (0, size, Some(Trap::IllegalInstruction(u32::from(half_word)))),
                |expanded| (expanded, size, None),
            )
        } else {
            let upper_paddr = f1.upper_paddr.unwrap_or_else(|| PhysAddr::new(phys_addr + 2));
            let upper_half = read_inst_half(state, upper_paddr.val());
            let full_inst = (upper_half as u32) << 16 | (half_word as u32);
            (full_inst, InstSize::Standard, None)
        };

        if let Some(t) = inst_trap {
            trace_trap!(state.uncore().trace_trap_enabled(&t);
                event = "decode-trap",
                stage = "F2",
                pc    = %Hex(f1.pc.val()),
                trap  = ?t,
                "F2: instruction decode trap"
            );
            output.push(IfIdEntry {
                pc: f1.pc,
                bits: 0,
                inst_size: step,
                predicted_target: f1.predicted_target,
                trap: Some(t),
                seq: f1.seq,
            });
            break;
        }

        trace_fetch!(state.uncore().kernel.config.general.trace_instructions;
            pc         = %Hex(f1.pc.val()),
            inst       = inst,
            inst_size  = step.as_u64(),
            compressed = is_compressed,
            "F2: decoded instruction"
        );

        output.push(IfIdEntry {
            pc: f1.pc,
            bits: inst,
            inst_size: step,
            predicted_target: f1.predicted_target,
            trap: None,
            seq: f1.seq,
        });
    }
}

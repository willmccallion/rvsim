//! Disassembly: an instruction word as assembly text, from what decode
//! makes of it, so an encoding prints exactly when the simulator runs it.

#![expect(
    clippy::cast_possible_truncation,
    reason = "disassembly slices fields and immediates out of an instruction word"
)]

mod scalar;
mod vector;

use crate::exec::decode::{DecodeFault, decode_inst};
use crate::isa::rvc;

/// The `vtype` instructions are decoded under: SEW 8 with LMUL 1/8, where
/// every register group decode checks is one register, so no legal
/// encoding is rejected for its alignment.
const VTYPE_SMALLEST_GROUPS: u64 = 0b101;

/// Disassembles the instruction `inst`, a 16-bit compressed one in its low
/// half or a 32-bit one, into text like `"add a0, a1, a2"`. An encoding
/// decode rejects prints as `"unknown (0x…)"`.
#[must_use]
pub fn disassemble(inst: u32) -> String {
    let word = if is_compressed(inst) { rvc::expand(inst as u16) } else { Some(inst) };
    let Some(word) = word else {
        return unknown(inst);
    };
    let decoded = decode_inst(word, VTYPE_SMALLEST_GROUPS);
    match decoded.fault {
        Some(DecodeFault::Breakpoint) => "ebreak".to_owned(),
        Some(_) => unknown(inst),
        None => match decoded.ctrl.vec_op {
            Some(op) => vector::disassemble(op, &decoded.ctrl, &decoded.fields),
            None => {
                scalar::disassemble(&decoded.ctrl, &decoded.fields).unwrap_or_else(|| unknown(inst))
            }
        },
    }
}

/// True for a 16-bit RVC encoding, whose low two bits are not `0b11`.
const fn is_compressed(inst: u32) -> bool {
    inst & 0b11 != 0b11
}

/// What an encoding decode rejects prints as.
fn unknown(inst: u32) -> String {
    format!("unknown ({inst:#010x})")
}

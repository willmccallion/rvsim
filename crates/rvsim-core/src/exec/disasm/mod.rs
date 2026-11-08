//! Disassembly: an instruction word as assembly text, from what decode
//! makes of it, so an encoding prints exactly when the simulator runs it.

mod scalar;
mod vector;

use crate::exec::decode::{DecodeFault, decode_inst};
use crate::isa::reg::{ArchReg, RegIdx};
use crate::isa::rvc;

/// The `vtype` instructions are decoded under: SEW 8 with LMUL 1/8, where
/// every register group decode checks is one register, so no legal
/// encoding is rejected for its alignment.
const VTYPE_SMALLEST_GROUPS: u64 = 0b101;

/// ABI names of x0–x31.
const XREG_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// ABI names of f0–f31.
const FREG_NAMES: [&str; 32] = [
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

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

/// The ABI name of integer register `idx`.
const fn xreg(idx: RegIdx) -> &'static str {
    XREG_NAMES[idx.as_usize()]
}

/// The ABI name of floating-point register `idx`.
const fn freg(idx: RegIdx) -> &'static str {
    FREG_NAMES[idx.as_usize()]
}

/// The ABI name of `reg`, in whichever register file it is.
const fn reg(reg: ArchReg) -> &'static str {
    match reg {
        ArchReg::Int(idx) => xreg(idx),
        ArchReg::Fp(idx) => freg(idx),
    }
}

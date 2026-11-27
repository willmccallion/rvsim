//! A `jalr` is a return when it jumps through a link register (`ra` or
//! `t0`) and does not write that same register, by the return-address
//! stack hints of the RISC-V unprivileged specification (table 2.1):
//! `jalr ra, t0` pops and then pushes, `jalr ra, ra` only pushes.

use crate::common::VirtAddr;
use crate::exec::decode::decode_inst;
use crate::exec::inst::Inst;
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::regs::{A0, RA, T0, X0};
use crate::uarch::bpred::ControlInst;

const PC: VirtAddr = VirtAddr::new(0x8000_0000);

/// Whether decode reads `jalr rd, 0(rs1)` as a return.
fn is_return(rd: RegIdx, rs1: RegIdx) -> bool {
    let word = InstructionBuilder::new().jalr(rd, rs1, 0).build();
    let decoded = decode_inst(word, 0);
    let inst = Inst {
        pc: PC,
        bits: word,
        rd: decoded.fields.rd,
        rs1: decoded.fields.rs1,
        ctrl: decoded.ctrl,
        ..Inst::default()
    };
    matches!(ControlInst::of(&inst), Some(ControlInst::IndirectJump { returns: true, .. }))
}

#[test]
fn a_jump_through_ra_writing_nothing_is_a_return() {
    assert!(is_return(X0, RA));
}

#[test]
fn a_jump_through_t0_writing_ra_is_a_return() {
    assert!(is_return(RA, T0));
}

#[test]
fn a_jump_through_ra_writing_ra_is_not_a_return() {
    assert!(!is_return(RA, RA));
}

#[test]
fn a_jump_through_another_register_is_not_a_return() {
    assert!(!is_return(X0, A0));
}

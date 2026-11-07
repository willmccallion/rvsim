//! The value an instruction leaving Memory2 writes to its destination.

use crate::exec::signals::{ControlFlow, ControlSignals};
use crate::isa::instruction::InstSize;
use crate::isa::op::AtomicOp;
use crate::uarch::pipeline::latches::Mem2WbEntry;

const PC: u64 = 0x8000_0000;
const ADDRESS: u64 = 0x8000_1000;

fn entry(ctrl: ControlSignals, alu: u64, load_data: u64) -> Mem2WbEntry {
    Mem2WbEntry {
        pc: PC,
        inst_size: InstSize::Standard,
        ctrl,
        alu,
        load_data,
        ..Default::default()
    }
}

#[test]
fn a_load_that_read_zero_writes_zero_not_its_address() {
    let load = ControlSignals { mem_read: true, ..Default::default() };

    assert_eq!(entry(load, ADDRESS, 0).written_value(), 0);
}

#[test]
fn an_atomic_writes_the_old_value_it_read() {
    let amo = ControlSignals { atomic_op: Some(AtomicOp::Add), ..Default::default() };

    assert_eq!(entry(amo, ADDRESS, 7).written_value(), 7);
}

#[test]
fn a_jump_writes_its_return_address() {
    let jump = ControlSignals { control_flow: ControlFlow::Jump, ..Default::default() };

    assert_eq!(entry(jump, ADDRESS, 0).written_value(), PC + 4);
}

#[test]
fn an_alu_op_writes_its_result() {
    assert_eq!(entry(ControlSignals::default(), 42, 0).written_value(), 42);
}

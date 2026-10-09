//! Reorder buffer tests.

#![allow(clippy::unwrap_used, unused_results)]

use super::*;
use crate::exec::signals::ControlSignals;
use crate::isa::csr::CsrAddr;
use crate::isa::reg::RegIdx;
use crate::uarch::pipeline::rename::prf::PhysReg;

fn make_ctrl(reg_write: bool, fp_reg_write: bool) -> ControlSignals {
    ControlSignals { reg_write, fp_reg_write, ..Default::default() }
}

fn alloc(rob: &mut Rob, pc: u64, rd: u8, ctrl: ControlSignals) -> Option<RobTag> {
    rob.allocate(
        pc,
        0,
        InstSize::Standard,
        RegIdx::new(rd),
        ctrl,
        PhysReg(0),
        PhysReg(0),
        crate::common::InstSeq::default(),
    )
}

#[test]
fn test_allocate_and_commit() {
    let mut rob = Rob::new(4);
    assert!(rob.is_empty());
    assert_eq!(rob.free_slots(), 4);

    let tag = rob
        .allocate(
            0x1000,
            0x13,
            InstSize::Standard,
            RegIdx::new(1),
            make_ctrl(true, false),
            PhysReg(0),
            PhysReg(0),
            crate::common::InstSeq::default(),
        )
        .unwrap();
    assert_eq!(rob.len(), 1);
    assert_eq!(rob.free_slots(), 3);

    // Can't commit while still Issued
    assert!(rob.commit_head().is_none());

    rob.complete(tag, 42);
    let entry = rob.commit_head().unwrap();
    assert_eq!(entry.pc, 0x1000);
    assert_eq!(entry.result, Some(42));
    assert_eq!(entry.state, RobState::Completed);
    assert!(rob.is_empty());
}

#[test]
fn test_full_rob() {
    let mut rob = Rob::new(2);
    let _t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    let _t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();
    assert!(rob.is_full());
    assert!(alloc(&mut rob, 0x1008, 3, make_ctrl(true, false)).is_none());
}

#[test]
fn test_in_order_commit() {
    let mut rob = Rob::new(4);
    let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    let t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();

    // Complete t2 first (out of order)
    rob.complete(t2, 200);
    // t1 is still Issued, so commit should fail
    assert!(rob.commit_head().is_none());

    // Now complete t1
    rob.complete(t1, 100);
    let e1 = rob.commit_head().unwrap();
    assert_eq!(e1.result, Some(100));

    let e2 = rob.commit_head().unwrap();
    assert_eq!(e2.result, Some(200));
}

#[test]
fn test_fault_commit() {
    let mut rob = Rob::new(4);
    let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    rob.fault(t1, Trap::IllegalInstruction(0), ExceptionStage::Decode);

    let entry = rob.commit_head().unwrap();
    assert_eq!(entry.state, RobState::Faulted);
    assert!(entry.trap.is_some());
}

#[test]
fn test_flush_all() {
    let mut rob = Rob::new(4);
    alloc(&mut rob, 0x1000, 1, make_ctrl(true, false));
    alloc(&mut rob, 0x1004, 2, make_ctrl(true, false));
    assert_eq!(rob.len(), 2);

    rob.flush_all();
    assert!(rob.is_empty());
    assert_eq!(rob.free_slots(), 4);
}

#[test]
fn test_flush_after() {
    let mut rob = Rob::new(8);
    let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    let _t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();
    let _t3 = alloc(&mut rob, 0x1008, 3, make_ctrl(true, false)).unwrap();
    assert_eq!(rob.len(), 3);

    rob.flush_after(t1);
    assert_eq!(rob.len(), 1);

    rob.complete(t1, 100);
    let entry = rob.commit_head().unwrap();
    assert_eq!(entry.pc, 0x1000);
}

#[test]
fn test_csr_update() {
    let mut rob = Rob::new(4);
    let tag = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    rob.set_csr_update(
        tag,
        CsrUpdate { addr: CsrAddr::from_u32(0x300), old_val: 10, new_val: 20, applied: false },
    );
    rob.complete(tag, 10);

    let entry = rob.commit_head().unwrap();
    let csr = entry.csr_update.unwrap();
    assert_eq!(csr.addr, CsrAddr::from_u32(0x300));
    assert_eq!(csr.new_val, 20);
}

#[test]
fn test_circular_wraparound() {
    let mut rob = Rob::new(2);

    // Fill and drain several times to test wraparound
    for i in 0..10 {
        let tag = alloc(&mut rob, i * 4, 1, make_ctrl(true, false)).unwrap();
        rob.complete(tag, i);
        let entry = rob.commit_head().unwrap();
        assert_eq!(entry.result, Some(i));
    }
}

/// Encode a FENCE instruction with given pred/succ bits.
/// FENCE encoding: opcode=0x0F, funct3=0, pred in bits[27:24], succ in bits[23:20].
fn encode_fence(pred: u8, succ: u8) -> u32 {
    0x0F | ((pred as u32 & 0xF) << 24) | ((succ as u32 & 0xF) << 20)
}

fn alloc_with_inst(rob: &mut Rob, inst: u32, ctrl: ControlSignals) -> Option<RobTag> {
    rob.allocate(
        0x1000,
        inst,
        InstSize::Standard,
        RegIdx::new(0),
        ctrl,
        PhysReg(0),
        PhysReg(0),
        crate::common::InstSeq::default(),
    )
}

#[test]
fn test_fence_pred_satisfied() {
    let mut rob = Rob::new(8);

    // Allocate: store (tag1), load (tag2), FENCE rw,rw (tag3)
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let fence_ctrl =
        ControlSignals { system_op: crate::isa::op::SystemOp::Fence, ..Default::default() };

    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    // FENCE rw,rw: pred=0b0011, succ=0b0011
    let t_fence = alloc_with_inst(&mut rob, encode_fence(0b0011, 0b0011), fence_ctrl).unwrap();

    // pred.r=true, pred.w=true: both older load and store must complete
    assert!(!rob.fence_pred_satisfied(t_fence, true, true));

    // Complete the store — still blocked by uncompleted load (pred.r)
    rob.complete(t_store, 0);
    assert!(!rob.fence_pred_satisfied(t_fence, true, true));

    // But pred.w only (FENCE w,*) would be satisfied now
    assert!(rob.fence_pred_satisfied(t_fence, false, true));

    // Complete the load — now fully satisfied
    rob.complete(t_load, 0);
    assert!(rob.fence_pred_satisfied(t_fence, true, true));
}

#[test]
fn test_has_fence_blocking() {
    let mut rob = Rob::new(8);

    // Allocate: FENCE w,r (tag1), load (tag2), store (tag3)
    let fence_ctrl =
        ControlSignals { system_op: crate::isa::op::SystemOp::Fence, ..Default::default() };
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };

    // FENCE w,r: pred=0b0001 (W), succ=0b0010 (R)
    let _t_fence = alloc_with_inst(&mut rob, encode_fence(0b0001, 0b0010), fence_ctrl).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

    // Load is blocked (succ.r = true)
    assert!(rob.has_fence_blocking(t_load, true, false));
    // Store is NOT blocked (succ.w = false)
    assert!(!rob.has_fence_blocking(t_store, false, true));
}

#[test]
fn an_acquire_atomic_holds_younger_loads_until_it_completes() {
    let mut rob = Rob::new(8);
    let acquire_ctrl = ControlSignals {
        atomic_op: Some(crate::isa::op::AtomicOp::Swap),
        acquire: true,
        mem_read: true,
        mem_write: true,
        ..Default::default()
    };
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };
    let t_amo = alloc_with_inst(&mut rob, 0, acquire_ctrl).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

    assert!(rob.has_fence_blocking(t_load, true, false));
    assert!(!rob.has_fence_blocking(t_store, false, true));
    rob.complete(t_amo, 0);
    assert!(!rob.has_fence_blocking(t_load, true, false));
}

#[test]
fn a_fence_waits_for_an_older_vector_store() {
    let mut rob = Rob::new(8);
    let vector_store =
        ControlSignals { vec_op: crate::isa::op::VectorOp::VStoreUnit, ..Default::default() };
    let fence_ctrl =
        ControlSignals { system_op: crate::isa::op::SystemOp::Fence, ..Default::default() };
    let t_store = alloc_with_inst(&mut rob, 0, vector_store).unwrap();
    let t_fence = alloc_with_inst(&mut rob, encode_fence(0b0001, 0b0001), fence_ctrl).unwrap();

    assert!(!rob.fence_pred_satisfied(t_fence, false, true));
    rob.complete(t_store, 0);
    assert!(rob.fence_pred_satisfied(t_fence, false, true));
}

#[test]
fn an_older_vector_store_is_a_memory_access_in_flight() {
    let mut rob = Rob::new(8);
    let vector_store =
        ControlSignals { vec_op: crate::isa::op::VectorOp::VStoreUnit, ..Default::default() };
    let vector_load =
        ControlSignals { vec_op: crate::isa::op::VectorOp::VLoadUnit, ..Default::default() };
    let t_store = alloc(&mut rob, 0, 0, vector_store).unwrap();
    let t_load = alloc(&mut rob, 4, 0, vector_load).unwrap();

    assert!(rob.has_older_memory_access(t_load));
    assert!(!rob.has_older_memory_access(t_store));
}

#[test]
fn an_older_arithmetic_instruction_is_no_memory_access() {
    let mut rob = Rob::new(8);
    let vector_load =
        ControlSignals { vec_op: crate::isa::op::VectorOp::VLoadUnit, ..Default::default() };
    alloc(&mut rob, 0, 1, make_ctrl(true, false)).unwrap();
    let t_load = alloc(&mut rob, 4, 0, vector_load).unwrap();

    assert!(!rob.has_older_memory_access(t_load));
}

#[test]
fn test_fence_tso_blocking() {
    let mut rob = Rob::new(8);

    // FENCE.TSO = FENCE rw,rw
    let fence_ctrl =
        ControlSignals { system_op: crate::isa::op::SystemOp::Fence, ..Default::default() };
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };

    // FENCE rw,rw: pred=0b0011, succ=0b0011
    let _t_fence = alloc_with_inst(&mut rob, encode_fence(0b0011, 0b0011), fence_ctrl).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

    // Both loads and stores are blocked
    assert!(rob.has_fence_blocking(t_load, true, false));
    assert!(rob.has_fence_blocking(t_store, false, true));
}

#[test]
fn test_control_outcome() {
    let mut rob = Rob::new(4);
    let t1 = alloc_with_inst(&mut rob, 0, ControlSignals::default()).unwrap();
    rob.set_control_outcome(t1, BpOutcome { taken: true, mispredicted: false }, Some(0x2000));

    let entry = rob.find_entry(t1).unwrap();
    assert!(entry.control_resolved);
    assert!(entry.bp_outcome.taken);
    assert_eq!(entry.bp_target, Some(0x2000));
    assert!(!entry.bp_outcome.mispredicted);
}

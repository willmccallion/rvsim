//! Reorder buffer tests.

#![allow(clippy::unwrap_used, unused_results)]

use super::*;
use crate::exec::signals::ControlSignals;
use crate::isa::csr::CsrAddr;
use crate::isa::op::{AmoOp, VecLoadOp, VecStoreOp};
use crate::isa::reg::RegIdx;

fn make_ctrl(reg_write: bool, fp_reg_write: bool) -> ControlSignals {
    ControlSignals { reg_write, fp_reg_write, ..Default::default() }
}

fn alloc(rob: &mut Rob, pc: u64, rd: u8, ctrl: ControlSignals) -> Option<RobTag> {
    rob.allocate(
        &crate::exec::inst::Inst {
            pc: VirtAddr::new(pc),
            bits: 0,
            size: InstSize::Standard,
            rd: RegIdx::new(rd),
            ctrl,
            ..Default::default()
        },
        crate::common::InstSeq::default(),
        crate::uarch::pipeline::rob::RenamedDst::NONE,
    )
}

#[test]
fn an_allocated_entry_commits_once_completed() {
    let mut rob = Rob::new(4);
    assert!(rob.is_empty());
    assert_eq!(rob.free_slots(), 4);

    let tag = rob
        .allocate(
            &crate::exec::inst::Inst {
                pc: VirtAddr::new(0x1000),
                bits: 0x13,
                size: InstSize::Standard,
                rd: RegIdx::new(1),
                ctrl: make_ctrl(true, false),
                ..Default::default()
            },
            crate::common::InstSeq::default(),
            crate::uarch::pipeline::rob::RenamedDst::NONE,
        )
        .unwrap();
    assert_eq!(rob.len(), 1);
    assert_eq!(rob.free_slots(), 3);

    // Can't commit while still Issued
    assert!(rob.commit_head().is_none());

    rob.complete(tag, 42);
    let entry = rob.commit_head().unwrap();
    assert_eq!(entry.pc, VirtAddr::new(0x1000));
    assert_eq!(entry.result, Some(42));
    assert_eq!(entry.state, RobState::Completed);
    assert!(rob.is_empty());
}

#[test]
fn a_full_rob_refuses_allocation() {
    let mut rob = Rob::new(2);
    let _t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    let _t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();
    assert!(rob.is_full());
    assert!(alloc(&mut rob, 0x1008, 3, make_ctrl(true, false)).is_none());
}

#[test]
fn entries_commit_in_order_even_when_completed_out_of_order() {
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
fn a_faulted_entry_commits_with_its_trap() {
    let mut rob = Rob::new(4);
    let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    rob.fault(t1, Trap::IllegalInstruction(0));

    let entry = rob.commit_head().unwrap();
    assert!(entry.trap().is_some());
}

#[test]
fn flush_all_empties_the_rob() {
    let mut rob = Rob::new(4);
    alloc(&mut rob, 0x1000, 1, make_ctrl(true, false));
    alloc(&mut rob, 0x1004, 2, make_ctrl(true, false));
    assert_eq!(rob.len(), 2);

    rob.flush_all();
    assert!(rob.is_empty());
    assert_eq!(rob.free_slots(), 4);
}

#[test]
fn flush_after_drops_every_younger_entry() {
    let mut rob = Rob::new(8);
    let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    let _t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();
    let _t3 = alloc(&mut rob, 0x1008, 3, make_ctrl(true, false)).unwrap();
    assert_eq!(rob.len(), 3);

    rob.flush_after(t1);
    assert_eq!(rob.len(), 1);

    rob.complete(t1, 100);
    let entry = rob.commit_head().unwrap();
    assert_eq!(entry.pc, VirtAddr::new(0x1000));
}

#[test]
fn a_csr_update_is_carried_to_commit() {
    let mut rob = Rob::new(4);
    let tag = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
    rob.set_csr_update(tag, CsrUpdate { addr: CsrAddr::from_u32(0x300), old_val: 10, new_val: 20 });
    rob.complete(tag, 10);

    let entry = rob.commit_head().unwrap();
    let csr = entry.csr_update.unwrap();
    assert_eq!(csr.addr, CsrAddr::from_u32(0x300));
    assert_eq!(csr.new_val, 20);
}

#[test]
fn allocation_wraps_around_after_repeated_fill_and_drain() {
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
    0x0F | ((u32::from(pred) & 0xF) << 24) | ((u32::from(succ) & 0xF) << 20)
}

/// Allocates a FENCE with the `pred` and `succ` sets.
fn alloc_fence(rob: &mut Rob, pred: u8, succ: u8) -> Option<RobTag> {
    let bits = encode_fence(pred, succ);
    let system_op = crate::isa::op::SystemOp::Fence(crate::isa::fence::Fence::decode(bits));
    alloc_with_inst(rob, bits, ControlSignals { system_op, ..Default::default() })
}

fn alloc_with_inst(rob: &mut Rob, inst: u32, ctrl: ControlSignals) -> Option<RobTag> {
    rob.allocate(
        &crate::exec::inst::Inst {
            pc: VirtAddr::new(0x1000),
            bits: inst,
            size: InstSize::Standard,
            rd: RegIdx::new(0),
            ctrl,
            ..Default::default()
        },
        crate::common::InstSeq::default(),
        crate::uarch::pipeline::rob::RenamedDst::NONE,
    )
}

#[test]
fn a_fence_waits_only_for_older_accesses_it_orders() {
    let mut rob = Rob::new(8);

    // Allocate: store (tag1), load (tag2), FENCE rw,rw (tag3)
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };

    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    // FENCE rw,rw: pred=0b0011, succ=0b0011
    let t_fence = alloc_fence(&mut rob, 0b0011, 0b0011).unwrap();

    // pred.r=true, pred.w=true: both older load and store must complete
    assert!(!rob.fence_pred_satisfied(t_fence, FenceSet::memory(true, true)));

    // Complete the store — still blocked by uncompleted load (pred.r)
    rob.complete(t_store, 0);
    assert!(!rob.fence_pred_satisfied(t_fence, FenceSet::memory(true, true)));

    // But pred.w only (FENCE w,*) would be satisfied now
    assert!(rob.fence_pred_satisfied(t_fence, FenceSet::memory(false, true)));

    // Complete the load — now fully satisfied
    rob.complete(t_load, 0);
    assert!(rob.fence_pred_satisfied(t_fence, FenceSet::memory(true, true)));
}

#[test]
fn a_fence_w_r_blocks_younger_loads_but_not_stores() {
    let mut rob = Rob::new(8);

    // Allocate: FENCE w,r (tag1), load (tag2), store (tag3)
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };

    // FENCE w,r: pred=0b0001 (W), succ=0b0010 (R)
    let _t_fence = alloc_fence(&mut rob, 0b0001, 0b0010).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

    // Load is blocked (succ.r = true)
    assert!(rob.has_fence_blocking(t_load, FenceSet::memory(true, false)));
    // Store is NOT blocked (succ.w = false)
    assert!(!rob.has_fence_blocking(t_store, FenceSet::memory(false, true)));
}

#[test]
fn an_acquire_atomic_holds_younger_loads_until_it_completes() {
    let mut rob = Rob::new(8);
    let acquire_ctrl = ControlSignals {
        atomic_op: Some(crate::isa::op::AtomicOp::Amo(AmoOp::Swap)),
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

    assert!(rob.has_fence_blocking(t_load, FenceSet::memory(true, false)));
    assert!(!rob.has_fence_blocking(t_store, FenceSet::memory(false, true)));
    rob.complete(t_amo, 0);
    assert!(!rob.has_fence_blocking(t_load, FenceSet::memory(true, false)));
}

#[test]
fn a_fence_waits_for_an_older_vector_store() {
    let mut rob = Rob::new(8);
    let vector_store = ControlSignals {
        vec_op: Some(crate::isa::op::VectorOp::Store(VecStoreOp::Unit)),
        ..Default::default()
    };
    let t_store = alloc_with_inst(&mut rob, 0, vector_store).unwrap();
    let t_fence = alloc_fence(&mut rob, 0b0001, 0b0001).unwrap();

    assert!(!rob.fence_pred_satisfied(t_fence, FenceSet::memory(false, true)));
    rob.complete(t_store, 0);
    assert!(rob.fence_pred_satisfied(t_fence, FenceSet::memory(false, true)));
}

#[test]
fn an_older_vector_store_is_a_memory_access_in_flight() {
    let mut rob = Rob::new(8);
    let vector_store = ControlSignals {
        vec_op: Some(crate::isa::op::VectorOp::Store(VecStoreOp::Unit)),
        ..Default::default()
    };
    let vector_load = ControlSignals {
        vec_op: Some(crate::isa::op::VectorOp::Load(VecLoadOp::Unit)),
        ..Default::default()
    };
    let t_store = alloc(&mut rob, 0, 0, vector_store).unwrap();
    let t_load = alloc(&mut rob, 4, 0, vector_load).unwrap();

    assert!(rob.has_older_memory_access(t_load));
    assert!(!rob.has_older_memory_access(t_store));
}

#[test]
fn an_older_arithmetic_instruction_is_no_memory_access() {
    let mut rob = Rob::new(8);
    let vector_load = ControlSignals {
        vec_op: Some(crate::isa::op::VectorOp::Load(VecLoadOp::Unit)),
        ..Default::default()
    };
    alloc(&mut rob, 0, 1, make_ctrl(true, false)).unwrap();
    let t_load = alloc(&mut rob, 4, 0, vector_load).unwrap();

    assert!(!rob.has_older_memory_access(t_load));
}

#[test]
fn a_fence_rw_rw_blocks_younger_loads_and_stores() {
    let mut rob = Rob::new(8);

    // FENCE.TSO = FENCE rw,rw
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };

    // FENCE rw,rw: pred=0b0011, succ=0b0011
    let _t_fence = alloc_fence(&mut rob, 0b0011, 0b0011).unwrap();
    let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
    let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

    // Both loads and stores are blocked
    assert!(rob.has_fence_blocking(t_load, FenceSet::memory(true, false)));
    assert!(rob.has_fence_blocking(t_store, FenceSet::memory(false, true)));
}

#[test]
fn a_control_outcome_is_recorded_on_its_entry() {
    let mut rob = Rob::new(4);
    let t1 = alloc_with_inst(&mut rob, 0, ControlSignals::default()).unwrap();
    rob.set_control_outcome(
        t1,
        BpOutcome { taken: true, mispredicted: false },
        Some(VirtAddr::new(0x2000)),
    );

    let entry = rob.find_entry(t1).unwrap();
    let control = entry.control.unwrap();
    assert!(control.outcome.taken);
    assert_eq!(control.target, Some(VirtAddr::new(0x2000)));
    assert!(!control.outcome.mispredicted);
}

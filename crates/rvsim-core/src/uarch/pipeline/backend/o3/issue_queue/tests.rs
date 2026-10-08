//! Issue queue tests.

use super::*;
use crate::exec::inst::Inst;
use crate::exec::signals::ControlSignals;
use crate::isa::instruction::InstSize;
use crate::isa::reg::RegIdx;
use crate::uarch::pipeline::latches::RenameIssueEntry;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rob::{HeadAtCycleStart, RobTag};

fn make_entry(rob_tag: u32) -> RenameIssueEntry {
    RenameIssueEntry {
        // NOP
        inst: Inst {
            pc: 0x1000 + (rob_tag as u64) * 4,
            bits: 0x13,
            size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            ctrl: ControlSignals::default(),
        },
        rob_tag: RobTag::new(rob_tag),
        rs1_tag: None,
        rs2_tag: None,
        rs3_tag: None,
        rs1_phys: PhysReg(0),
        rs2_phys: PhysReg(0),
        rs3_phys: PhysReg(0),
        rd_phys: PhysReg(0),
        trap: None,
        exception_stage: None,
        pred_taken: false,
        pred_target: 0,
        seq: crate::common::InstSeq::default(),
        vs1_phys: [crate::uarch::pipeline::rename::vec_prf::VecPhysReg::ZERO; 8],
        vs2_phys: [crate::uarch::pipeline::rename::vec_prf::VecPhysReg::ZERO; 8],
        vs3_phys: [crate::uarch::pipeline::rename::vec_prf::VecPhysReg::ZERO; 8],
        vd_phys: [crate::uarch::pipeline::rename::vec_prf::VecPhysReg::ZERO; 8],
        vec_src1_count: 0,
        vec_src2_count: 0,
        vec_src3_count: 0,
        mask_phys: crate::uarch::pipeline::rename::vec_prf::VecPhysReg::ZERO,
        vec_vtype: 0,
        vec_vl: 0,
        vec_vstart: 0,
        vec_vxrm: 0,
        vec_frm: 0,
    }
}

/// Selects at cycle 0 with more address units than any test issues.
fn select(
    iq: &mut IssueQueue,
    width: usize,
    store_buffer: &StoreBuffer,
    rob: &Rob,
    load_ports: usize,
    store_ports: usize,
) -> Vec<SelectedEntry> {
    use crate::config::FuConfig;
    let units = FuPool::new(&FuConfig { num_mem: 8, ..FuConfig::default() });
    let budget = IssueBudget {
        width,
        load_ports,
        store_ports,
        units: &units,
        now: 0,
        memory_blocked: false,
        head: HeadAtCycleStart::latch(rob),
    };
    iq.select(&budget, store_buffer, rob).entries
}

/// A ready entry for `rob_tag` executing `ctrl`.
fn ready_entry(rob_tag: u32, ctrl: ControlSignals) -> IssueQueueEntry {
    let base = make_entry(rob_tag);
    IssueQueueEntry {
        entry: RenameIssueEntry { inst: Inst { ctrl, ..base.inst }, ..base },
        src1: ready_operand(0),
        src2: ready_operand(0),
        src3: ready_operand(0),
        vec_src1: VecOperandState::default(),
        vec_src2: VecOperandState::default(),
        vec_src3: VecOperandState::default(),
        mem_dep: MemDepState::None,
        mask_phys: VecPhysReg::ZERO,
        mask_ready: true,
        needs_mask: false,
        store_issue: StoreIssue::Whole,
    }
}

#[test]
fn a_ready_op_whose_unit_is_busy_lets_a_younger_op_issue_in_its_place() {
    use crate::config::FuConfig;
    use crate::isa::op::AluOp;
    let mut units = FuPool::new(&FuConfig { num_int_div: 1, ..FuConfig::default() });
    let busy_divider = units.free_unit(FuType::IntDiv, 0).expect("a divider");
    let _ = units.acquire(busy_divider, 0);
    let mut iq = IssueQueue::new(8);
    iq.slots[0] = Some(ready_entry(1, ControlSignals { alu: AluOp::Div, ..Default::default() }));
    iq.slots[1] = Some(ready_entry(2, ControlSignals::default()));
    iq.count = 2;
    let budget = IssueBudget {
        width: 1,
        load_ports: 1,
        store_ports: 1,
        units: &units,
        now: 1,
        memory_blocked: false,
        head: HeadAtCycleStart::default(),
    };

    let selection = iq.select(&budget, &StoreBuffer::new(4), &Rob::new(8));

    let issued: Vec<u32> = selection.entries.iter().map(|e| e.entry.rob_tag.raw()).collect();
    assert_eq!((issued, selection.unit_stalls), (vec![2], 1));
}

#[test]
fn a_blocked_memory_pipeline_holds_loads_but_not_alu_ops() {
    let units = FuPool::new(&crate::config::FuConfig::default());
    let mut iq = IssueQueue::new(8);
    iq.slots[0] = Some(ready_entry(1, ControlSignals { mem_read: true, ..Default::default() }));
    iq.slots[1] = Some(ready_entry(2, ControlSignals::default()));
    iq.count = 2;
    let budget = IssueBudget {
        width: 1,
        load_ports: 1,
        store_ports: 1,
        units: &units,
        now: 0,
        memory_blocked: true,
        head: HeadAtCycleStart::default(),
    };

    let selection = iq.select(&budget, &StoreBuffer::new(4), &Rob::new(8));

    let issued: Vec<u32> = selection.entries.iter().map(|e| e.entry.rob_tag.raw()).collect();
    assert_eq!(issued, vec![2]);
}

fn ready_operand(value: u64) -> OperandState {
    OperandState::ready(PhysReg(0), value)
}

fn not_ready_operand_phys(phys: PhysReg) -> OperandState {
    OperandState::not_ready(phys)
}

#[test]
fn test_new_empty() {
    let iq = IssueQueue::new(16);
    assert!(iq.is_empty());
    assert_eq!(iq.available_slots(), 16);
}

#[test]
fn test_dispatch_and_select_ready() {
    let mut iq = IssueQueue::new(16);

    // Manually insert a ready entry
    iq.slots[0] = Some(IssueQueueEntry {
        entry: make_entry(1),
        src1: ready_operand(42),
        src2: ready_operand(10),
        src3: ready_operand(0),
        vec_src1: VecOperandState::default(),
        vec_src2: VecOperandState::default(),
        vec_src3: VecOperandState::default(),
        mem_dep: MemDepState::None,
        mask_phys: VecPhysReg::ZERO,
        mask_ready: true,
        needs_mask: false,
        store_issue: StoreIssue::Whole,
    });
    iq.count = 1;

    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].entry.rob_tag.raw(), 1);
    assert_eq!(selected[0].entry.inst.rv1, 42);
    assert_eq!(selected[0].entry.inst.rv2, 10);
    assert!(iq.is_empty());
}

#[test]
fn test_wakeup_phys_chain() {
    let mut iq = IssueQueue::new(16);
    let p5 = PhysReg(5);

    // Entry depends on phys reg 5
    let entry = make_entry(10);
    iq.slots[0] = Some(IssueQueueEntry {
        entry,
        src1: not_ready_operand_phys(p5),
        src2: ready_operand(0),
        src3: ready_operand(0),
        vec_src1: VecOperandState::default(),
        vec_src2: VecOperandState::default(),
        vec_src3: VecOperandState::default(),
        mem_dep: MemDepState::None,
        mask_phys: VecPhysReg::ZERO,
        mask_ready: true,
        needs_mask: false,
        store_issue: StoreIssue::Whole,
    });
    iq.count = 1;

    // Not ready yet
    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
    assert_eq!(selected.len(), 0);

    // Wakeup with phys reg 5
    iq.wakeup_phys(p5, 999);

    // Now should be selectable
    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].entry.inst.rv1, 999);
}

#[test]
fn test_oldest_first_select() {
    let mut iq = IssueQueue::new(16);

    // Insert entries with tags 3, 1, 2 in random slot order
    for (slot, tag) in [(2, 3u32), (0, 1), (1, 2)] {
        iq.slots[slot] = Some(IssueQueueEntry {
            entry: make_entry(tag),
            src1: ready_operand(tag as u64),
            src2: ready_operand(0),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
            store_issue: StoreIssue::Whole,
        });
    }
    iq.count = 3;

    // Select width=2 should get tags 1 and 2 (oldest first)
    let selected = select(&mut iq, 2, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].entry.rob_tag.raw(), 1);
    assert_eq!(selected[1].entry.rob_tag.raw(), 2);
    assert_eq!(iq.len(), 1);

    // Remaining is tag 3
    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].entry.rob_tag.raw(), 3);
}

#[test]
fn test_flush() {
    let mut iq = IssueQueue::new(16);
    iq.slots[0] = Some(IssueQueueEntry {
        entry: make_entry(1),
        src1: OperandState::default(),
        src2: OperandState::default(),
        src3: OperandState::default(),
        vec_src1: VecOperandState::default(),
        vec_src2: VecOperandState::default(),
        vec_src3: VecOperandState::default(),
        mem_dep: MemDepState::None,
        mask_phys: VecPhysReg::ZERO,
        mask_ready: true,
        needs_mask: false,
        store_issue: StoreIssue::Whole,
    });
    iq.slots[5] = Some(IssueQueueEntry {
        entry: make_entry(2),
        src1: OperandState::default(),
        src2: OperandState::default(),
        src3: OperandState::default(),
        vec_src1: VecOperandState::default(),
        vec_src2: VecOperandState::default(),
        vec_src3: VecOperandState::default(),
        mem_dep: MemDepState::None,
        mask_phys: VecPhysReg::ZERO,
        mask_ready: true,
        needs_mask: false,
        store_issue: StoreIssue::Whole,
    });
    iq.count = 2;

    iq.flush();
    assert!(iq.is_empty());
    assert_eq!(iq.available_slots(), 16);
}

#[test]
fn test_flush_after() {
    let mut iq = IssueQueue::new(16);
    for (slot, tag) in [(0, 1u32), (1, 2), (2, 3), (3, 4)] {
        iq.slots[slot] = Some(IssueQueueEntry {
            entry: make_entry(tag),
            src1: OperandState::default(),
            src2: OperandState::default(),
            src3: OperandState::default(),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
            store_issue: StoreIssue::Whole,
        });
    }
    iq.count = 4;

    // Keep tags <= 2
    iq.flush_after(RobTag::new(2));
    assert_eq!(iq.len(), 2);

    let snap = iq.queue_snapshot();
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].rob_tag.raw(), 1);
    assert_eq!(snap[1].rob_tag.raw(), 2);
}

#[test]
fn test_queue_snapshot_sorted() {
    let mut iq = IssueQueue::new(16);
    // Insert in reverse order
    for (slot, tag) in [(0, 5u32), (1, 3), (2, 1)] {
        iq.slots[slot] = Some(IssueQueueEntry {
            entry: make_entry(tag),
            src1: OperandState::default(),
            src2: OperandState::default(),
            src3: OperandState::default(),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
            store_issue: StoreIssue::Whole,
        });
    }
    iq.count = 3;

    let snap = iq.queue_snapshot();
    assert_eq!(snap.len(), 3);
    assert_eq!(snap[0].rob_tag.raw(), 1);
    assert_eq!(snap[1].rob_tag.raw(), 3);
    assert_eq!(snap[2].rob_tag.raw(), 5);
}

#[test]
fn a_vector_load_waits_behind_an_incomplete_acquire_atomic() {
    let mut rob = Rob::new(8);
    let acquire = ControlSignals {
        atomic_op: Some(crate::isa::op::AtomicOp::Swap),
        acquire: true,
        mem_read: true,
        mem_write: true,
        ..Default::default()
    };
    let vector_load =
        ControlSignals { vec_op: crate::isa::op::VectorOp::VLoadUnit, ..Default::default() };
    let alloc = |rob: &mut Rob, ctrl| {
        rob.allocate(
            0,
            0,
            InstSize::Standard,
            RegIdx::new(0),
            ctrl,
            PhysReg(0),
            PhysReg(0),
            crate::common::InstSeq::default(),
        )
        .unwrap()
    };
    let amo_tag = alloc(&mut rob, acquire);
    let load_tag = alloc(&mut rob, vector_load);
    let mut iq = IssueQueue::new(4);
    let mut entry = make_entry(load_tag.raw());
    entry.inst.ctrl = vector_load;
    iq.slots[0] = Some(IssueQueueEntry {
        entry,
        src1: ready_operand(0),
        src2: ready_operand(0),
        src3: ready_operand(0),
        vec_src1: VecOperandState::default(),
        vec_src2: VecOperandState::default(),
        vec_src3: VecOperandState::default(),
        mem_dep: MemDepState::None,
        mask_phys: VecPhysReg::ZERO,
        mask_ready: true,
        needs_mask: false,
        store_issue: StoreIssue::Whole,
    });
    iq.count = 1;

    assert!(select(&mut iq, 4, &StoreBuffer::new(4), &rob, 2, 1).is_empty());
    rob.complete(amo_tag, 0);
    assert_eq!(select(&mut iq, 4, &StoreBuffer::new(4), &rob, 2, 1).len(), 1);
}

#[test]
fn test_port_limits() {
    let mut iq = IssueQueue::new(16);

    // Insert 3 loads (tags 1, 2, 3) and 2 stores (tags 4, 5), all ready
    for (slot, tag, is_load, is_store) in [
        (0, 1u32, true, false),
        (1, 2, true, false),
        (2, 3, true, false),
        (3, 4, false, true),
        (4, 5, false, true),
    ] {
        let mut entry = make_entry(tag);
        entry.inst.ctrl.mem_read = is_load;
        entry.inst.ctrl.mem_write = is_store;
        iq.slots[slot] = Some(IssueQueueEntry {
            entry,
            src1: ready_operand(0),
            src2: ready_operand(0),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
            store_issue: StoreIssue::Whole,
        });
    }
    iq.count = 5;

    // With load_ports=2, store_ports=1, width=4: should get 2 loads + 1 store = 3
    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), 2, 1);
    assert_eq!(selected.len(), 3);
    // Oldest first: tags 1 (load), 2 (load), 4 (store)
    assert_eq!(selected[0].entry.rob_tag.raw(), 1);
    assert!(selected[0].entry.inst.ctrl.mem_read);
    assert_eq!(selected[1].entry.rob_tag.raw(), 2);
    assert!(selected[1].entry.inst.ctrl.mem_read);
    assert_eq!(selected[2].entry.rob_tag.raw(), 4);
    assert!(selected[2].entry.inst.ctrl.mem_write);

    // Remaining: tag 3 (load), tag 5 (store)
    assert_eq!(iq.len(), 2);

    // Next cycle: should get remaining load + store
    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), 2, 1);
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].entry.rob_tag.raw(), 3);
    assert_eq!(selected[1].entry.rob_tag.raw(), 5);
    assert!(iq.is_empty());
}

/// Four tags a ROB hands out across the wrap, oldest first.
fn tags_across_the_wrap() -> Vec<RobTag> {
    let mut rob = Rob::new(4);
    rob.start_tags_at(RobTag::new(u32::MAX - 1));
    (0..4)
        .map(|_| {
            rob.allocate(
                0,
                0,
                InstSize::Standard,
                RegIdx::new(0),
                ControlSignals::default(),
                PhysReg(0),
                PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap()
        })
        .collect()
}

/// A queue holding `tags`, the ones allocated after the wrap in the
/// lowest slots.
fn queue_across_the_wrap(tags: &[RobTag]) -> IssueQueue {
    let mut iq = IssueQueue::new(8);
    for (slot, tag) in tags.iter().rev().enumerate() {
        iq.slots[slot] = Some(ready_entry(tag.raw(), ControlSignals::default()));
    }
    iq.count = tags.len();
    iq
}

#[test]
fn select_takes_the_oldest_ready_entries_across_the_tag_wrap() {
    let tags = tags_across_the_wrap();
    let mut iq = queue_across_the_wrap(&tags);

    let selected = select(&mut iq, 2, &StoreBuffer::new(4), &Rob::new(8), usize::MAX, usize::MAX);

    let issued: Vec<RobTag> = selected.iter().map(|e| e.entry.rob_tag).collect();
    assert_eq!(issued, tags[..2]);
}

#[test]
fn the_issue_hold_reports_the_oldest_entry_across_the_tag_wrap() {
    let tags = tags_across_the_wrap();
    let mut iq = queue_across_the_wrap(&tags);
    let oldest = iq.slots.iter_mut().flatten().find(|iq| iq.entry.rob_tag == tags[0]).unwrap();
    oldest.src1 = not_ready_operand_phys(PhysReg(5));
    let units = FuPool::new(&crate::config::FuConfig::default());
    let budget = IssueBudget {
        width: 1,
        load_ports: 1,
        store_ports: 1,
        units: &units,
        now: 0,
        memory_blocked: false,
        head: HeadAtCycleStart::default(),
    };

    let selection = iq.select(&budget, &StoreBuffer::new(4), &Rob::new(8));

    assert_eq!(selection.oldest, Some(IssueHold::Operands));
}

#[test]
fn the_queue_snapshot_lists_entries_oldest_first_across_the_tag_wrap() {
    let tags = tags_across_the_wrap();
    let iq = queue_across_the_wrap(&tags);

    let snapshot: Vec<RobTag> = iq.queue_snapshot().iter().map(|e| e.rob_tag).collect();

    assert_eq!(snapshot, tags);
}

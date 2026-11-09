//! Issue queue tests. Entries enter through `free_slot` and `fill`, as
//! dispatch puts them there.

use super::*;
use crate::common::{InstSeq, PhysAddr, VirtAddr};
use crate::config::{Config, FuConfig};
use crate::exec::inst::Inst;
use crate::exec::signals::ControlSignals;
use crate::isa::fence::Fence;
use crate::isa::op::{
    AluOp, AmoOp, AtomicOp, DivOp, IntAluOp, MemWidth, SystemOp, VecLoadOp, VectorOp,
};
use crate::isa::reg::RegIdx;
use crate::sim::components::{CacheId, PipelineId};
use crate::system::SystemState;
use crate::uarch::pipeline::backend::o3::O3Engine;
use crate::uarch::pipeline::rob::RenamedDst;

/// A NOP writing x1, tagged `rob_tag`.
fn make_entry(rob_tag: u32) -> O3Entry {
    RenameIssueEntry {
        inst: Inst {
            pc: VirtAddr::new(0x1000 + u64::from(rob_tag) * 4),
            bits: 0x13,
            rd: RegIdx::new(1),
            ..Inst::default()
        },
        rob_tag: RobTag::new(rob_tag),
        ..RenameIssueEntry::default()
    }
}

fn ready_operand(value: u64) -> OperandState {
    OperandState::ready(PhysReg::ZERO, value)
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
        mask_ready: true,
        store_issue: StoreIssue::Whole,
    }
}

/// A ready ALU entry for `rob_tag`.
fn ready_nop(rob_tag: u32) -> IssueQueueEntry {
    ready_entry(rob_tag, ControlSignals::default())
}

/// A queue of `capacity` slots that `entries` were dispatched into, in order.
fn queue_of(capacity: usize, entries: impl IntoIterator<Item = IssueQueueEntry>) -> IssueQueue {
    let mut iq = IssueQueue::new(capacity);
    for entry in entries {
        iq.free_slot().expect("the queue has room").fill(&mut iq, entry);
    }
    iq
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
    selection_under(iq, &budget, store_buffer, rob).entries
}

/// What `iq` selects under `budget`, in a fresh selection.
fn selection_under(
    iq: &mut IssueQueue,
    budget: &IssueBudget<'_>,
    store_buffer: &StoreBuffer,
    rob: &Rob,
) -> Selection {
    let mut selection = Selection::default();
    iq.select(budget, store_buffer, rob, &mut selection);
    selection
}

/// One ALU slot, one load and one store port, from `units`.
fn single_issue(units: &FuPool, now: u64, memory_blocked: bool) -> IssueBudget<'_> {
    IssueBudget {
        width: 1,
        load_ports: 1,
        store_ports: 1,
        units,
        now,
        memory_blocked,
        head: HeadAtCycleStart::default(),
    }
}

fn issued_tags(selected: &[SelectedEntry]) -> Vec<u32> {
    selected.iter().map(|e| e.entry.rob_tag.raw()).collect()
}

/// Allocates an instruction executing `ctrl` in `rob`.
fn allocate(rob: &mut Rob, ctrl: ControlSignals) -> RobTag {
    rob.allocate(&Inst { ctrl, ..Inst::default() }, InstSeq::default(), RenamedDst::NONE)
        .expect("the ROB has room")
}

#[test]
fn a_ready_op_whose_unit_is_busy_lets_a_younger_op_issue_in_its_place() {
    let mut units = FuPool::new(&FuConfig { num_int_div: 1, ..FuConfig::default() });
    let busy_divider = units.free_unit(FuType::IntDiv, 0).expect("a divider");
    let _ = units.acquire(busy_divider, 0, RobTag::new(9));
    let divide =
        ControlSignals { alu: AluOp::Int(IntAluOp::Div(DivOp::Div)), ..Default::default() };
    let mut iq = queue_of(8, [ready_entry(1, divide), ready_nop(2)]);

    let selection = selection_under(
        &mut iq,
        &single_issue(&units, 1, false),
        &StoreBuffer::new(4),
        &Rob::new(8),
    );

    assert_eq!((issued_tags(&selection.entries), selection.unit_stalls), (vec![2], 1));
}

#[test]
fn a_blocked_memory_pipeline_holds_loads_but_not_alu_ops() {
    let units = FuPool::new(&FuConfig::default());
    let load = ControlSignals { mem_read: true, ..Default::default() };
    let mut iq = queue_of(8, [ready_entry(1, load), ready_nop(2)]);

    let selection = selection_under(
        &mut iq,
        &single_issue(&units, 0, true),
        &StoreBuffer::new(4),
        &Rob::new(8),
    );

    assert_eq!(issued_tags(&selection.entries), vec![2]);
}

#[test]
fn a_new_queue_is_empty_with_every_slot_free() {
    let iq = IssueQueue::new(16);
    assert!(iq.is_empty());
    assert_eq!(iq.available_slots(), 16);
}

#[test]
fn a_dispatched_entry_takes_a_slot() {
    let iq = queue_of(4, [ready_nop(1)]);

    assert_eq!((iq.len(), iq.available_slots()), (1, 3));
}

#[test]
fn a_full_queue_has_no_free_slot() {
    let iq = queue_of(2, [ready_nop(1), ready_nop(2)]);

    assert!(iq.free_slot().is_none());
}

#[test]
fn a_ready_entry_issues_with_its_operand_values() {
    let entry =
        IssueQueueEntry { src1: ready_operand(42), src2: ready_operand(10), ..ready_nop(1) };
    let mut iq = queue_of(16, [entry]);

    let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);

    assert_eq!(issued_tags(&selected), vec![1]);
    assert_eq!((selected[0].entry.inst.rv1, selected[0].entry.inst.rv2), (42, 10));
    assert!(iq.is_empty());
}

#[test]
fn a_waiting_entry_issues_with_the_woken_value() {
    let p5 = PhysReg::from_index(5);
    let mut iq =
        queue_of(16, [IssueQueueEntry { src1: OperandState::not_ready(p5), ..ready_nop(10) }]);
    let rob = Rob::new(64);
    let store_buffer = StoreBuffer::new(16);

    let before = select(&mut iq, 4, &store_buffer, &rob, usize::MAX, usize::MAX);
    iq.wakeup_phys(p5, 999);
    let after = select(&mut iq, 4, &store_buffer, &rob, usize::MAX, usize::MAX);

    assert!(before.is_empty());
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].entry.inst.rv1, 999);
}

#[test]
fn select_issues_the_oldest_ready_entries_up_to_the_width() {
    let entries = [3u32, 1, 2]
        .map(|tag| IssueQueueEntry { src1: ready_operand(u64::from(tag)), ..ready_nop(tag) });
    let mut iq = queue_of(16, entries);
    let rob = Rob::new(64);
    let store_buffer = StoreBuffer::new(16);

    let first = select(&mut iq, 2, &store_buffer, &rob, usize::MAX, usize::MAX);
    let left_after_first = iq.len();
    let second = select(&mut iq, 4, &store_buffer, &rob, usize::MAX, usize::MAX);

    assert_eq!(issued_tags(&first), vec![1, 2]);
    assert_eq!(left_after_first, 1);
    assert_eq!(issued_tags(&second), vec![3]);
}

#[test]
fn flush_empties_the_queue() {
    let mut iq = queue_of(16, [ready_nop(1), ready_nop(2)]);

    iq.flush();

    assert!(iq.is_empty());
    assert_eq!(iq.available_slots(), 16);
}

#[test]
fn flush_after_keeps_only_entries_up_to_the_tag() {
    let mut iq = queue_of(16, [1, 2, 3, 4].map(ready_nop));

    iq.flush_after(RobTag::new(2));

    let kept: Vec<u32> = iq.queue_snapshot().iter().map(|e| e.rob_tag.raw()).collect();
    assert_eq!(kept, [1, 2]);
    assert_eq!(iq.len(), 2);
}

#[test]
fn the_queue_snapshot_lists_entries_oldest_first() {
    let iq = queue_of(16, [5, 3, 1].map(ready_nop));

    let snapshot: Vec<u32> = iq.queue_snapshot().iter().map(|e| e.rob_tag.raw()).collect();

    assert_eq!(snapshot, [1, 3, 5]);
}

#[test]
fn a_vector_load_waits_behind_an_incomplete_acquire_atomic() {
    let mut rob = Rob::new(8);
    let acquire = ControlSignals {
        atomic_op: Some(AtomicOp::Amo(AmoOp::Swap)),
        acquire: true,
        mem_read: true,
        mem_write: true,
        ..Default::default()
    };
    let vector_load =
        ControlSignals { vec_op: Some(VectorOp::Load(VecLoadOp::Unit)), ..Default::default() };
    let amo_tag = allocate(&mut rob, acquire);
    let load_tag = allocate(&mut rob, vector_load);
    let mut iq = queue_of(4, [ready_entry(load_tag.raw(), vector_load)]);

    let before = select(&mut iq, 4, &StoreBuffer::new(4), &rob, 2, 1);
    rob.complete(amo_tag, 0);
    let after = select(&mut iq, 4, &StoreBuffer::new(4), &rob, 2, 1);

    assert!(before.is_empty());
    assert_eq!(issued_tags(&after), vec![load_tag.raw()]);
}

#[test]
fn select_issues_no_more_loads_and_stores_than_their_ports() {
    let load = ControlSignals { mem_read: true, ..Default::default() };
    let store = ControlSignals { mem_write: true, ..Default::default() };
    let entries = [(1, load), (2, load), (3, load), (4, store), (5, store)]
        .map(|(tag, ctrl)| ready_entry(tag, ctrl));
    let mut iq = queue_of(16, entries);
    let rob = Rob::new(64);
    let store_buffer = StoreBuffer::new(16);

    let first = select(&mut iq, 4, &store_buffer, &rob, 2, 1);
    let second = select(&mut iq, 4, &store_buffer, &rob, 2, 1);

    assert_eq!(issued_tags(&first), vec![1, 2, 4], "two loads and a store");
    assert_eq!(issued_tags(&second), vec![3, 5]);
    assert!(iq.is_empty());
}

/// `fence pred, succ` with each set given as its four IORW bits.
fn fence(pred: u32, succ: u32) -> ControlSignals {
    let bits = 0x0F | (pred << 24) | (succ << 20);
    ControlSignals { system_op: SystemOp::Fence(Fence::decode(bits)), ..Default::default() }
}

/// Retires the oldest instruction.
fn retire_head(rob: &mut Rob) {
    let head = rob.peek_head().expect("the ROB holds an instruction").tag;
    rob.complete(head, 0);
    let _ = rob.commit_head();
}

/// What one cycle issues from `iq`, given `rob` and `store_buffer`.
fn issue_one(iq: &mut IssueQueue, rob: &Rob, store_buffer: &StoreBuffer) -> Vec<u32> {
    issued_tags(&select(iq, 1, store_buffer, rob, 1, 1))
}

#[test]
fn a_csr_access_waits_until_it_is_the_oldest_instruction() {
    let mut rob = Rob::new(8);
    let _older = allocate(&mut rob, ControlSignals::default());
    let csr_access = ControlSignals { system_op: SystemOp::Csr, ..Default::default() };
    let tag = allocate(&mut rob, csr_access);
    let mut iq = queue_of(4, [ready_entry(tag.raw(), csr_access)]);
    let store_buffer = StoreBuffer::new(4);

    let behind_an_older_op = issue_one(&mut iq, &rob, &store_buffer);
    retire_head(&mut rob);
    let at_the_head = issue_one(&mut iq, &rob, &store_buffer);

    assert!(behind_an_older_op.is_empty());
    assert_eq!(at_the_head, [tag.raw()]);
}

#[test]
fn an_amo_waits_until_it_is_the_oldest_instruction() {
    let mut rob = Rob::new(8);
    let _older = allocate(&mut rob, ControlSignals::default());
    let amo = ControlSignals {
        atomic_op: Some(AtomicOp::Amo(AmoOp::Add)),
        mem_read: true,
        mem_write: true,
        ..Default::default()
    };
    let tag = allocate(&mut rob, amo);
    let mut iq = queue_of(4, [ready_entry(tag.raw(), amo)]);
    let store_buffer = StoreBuffer::new(4);

    let behind_an_older_op = issue_one(&mut iq, &rob, &store_buffer);
    retire_head(&mut rob);
    let at_the_head = issue_one(&mut iq, &rob, &store_buffer);

    assert!(behind_an_older_op.is_empty());
    assert_eq!(at_the_head, [tag.raw()]);
}

#[test]
fn a_load_told_to_wait_for_all_stores_waits_for_an_unresolved_older_one() {
    let mut rob = Rob::new(8);
    let store = allocate(&mut rob, ControlSignals { mem_write: true, ..Default::default() });
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let load = allocate(&mut rob, load_ctrl);
    let entry =
        IssueQueueEntry { mem_dep: MemDepState::WaitAll, ..ready_entry(load.raw(), load_ctrl) };
    let mut iq = queue_of(4, [entry]);
    let mut store_buffer = StoreBuffer::new(4);
    let _ = store_buffer.allocate(store, MemWidth::Word);

    let unresolved = issue_one(&mut iq, &rob, &store_buffer);
    let address = PhysAddr::new(0x8000_0000);
    store_buffer.resolve_address(store, VirtAddr::new(0x8000_0000), address);
    let resolved = issue_one(&mut iq, &rob, &store_buffer);

    assert!(unresolved.is_empty());
    assert_eq!(resolved, [load.raw()]);
}

#[test]
fn a_load_waits_behind_an_older_fence_that_orders_reads() {
    let mut rob = Rob::new(8);
    let _fence = allocate(&mut rob, fence(0b0010, 0b0010));
    let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
    let load = allocate(&mut rob, load_ctrl);
    let mut iq = queue_of(4, [ready_entry(load.raw(), load_ctrl)]);
    let store_buffer = StoreBuffer::new(4);

    let behind_the_fence = issue_one(&mut iq, &rob, &store_buffer);
    retire_head(&mut rob);
    let after_it = issue_one(&mut iq, &rob, &store_buffer);

    assert!(behind_the_fence.is_empty());
    assert_eq!(after_it, [load.raw()]);
}

#[test]
fn an_alu_op_passes_an_older_fence() {
    let mut rob = Rob::new(8);
    let _fence = allocate(&mut rob, fence(0b0011, 0b0011));
    let alu = allocate(&mut rob, ControlSignals::default());
    let mut iq = queue_of(4, [ready_nop(alu.raw())]);

    assert_eq!(issue_one(&mut iq, &rob, &StoreBuffer::new(4)), [alu.raw()]);
}

#[test]
fn a_fence_waits_for_the_older_loads_it_orders() {
    let mut rob = Rob::new(8);
    let load = allocate(&mut rob, ControlSignals { mem_read: true, ..Default::default() });
    let fence_ctrl = fence(0b0010, 0b0011);
    let tag = allocate(&mut rob, fence_ctrl);
    let mut iq = queue_of(4, [ready_entry(tag.raw(), fence_ctrl)]);
    let store_buffer = StoreBuffer::new(4);

    let while_the_load_is_out = issue_one(&mut iq, &rob, &store_buffer);
    rob.complete(load, 0);
    let once_it_completes = issue_one(&mut iq, &rob, &store_buffer);

    assert!(while_the_load_is_out.is_empty());
    assert_eq!(once_it_completes, [tag.raw()]);
}

/// Four tags a ROB hands out across the wrap, oldest first.
fn tags_across_the_wrap() -> Vec<RobTag> {
    let mut rob = Rob::new(4);
    rob.start_tags_at(RobTag::new(u32::MAX - 1));
    (0..4).map(|_| allocate(&mut rob, ControlSignals::default())).collect()
}

/// `entries` for `tags` dispatched youngest first, so the slots run
/// against age.
fn queue_across_the_wrap(tags: &[RobTag], entry: impl Fn(RobTag) -> IssueQueueEntry) -> IssueQueue {
    queue_of(8, tags.iter().rev().map(|&tag| entry(tag)))
}

#[test]
fn select_takes_the_oldest_ready_entries_across_the_tag_wrap() {
    let tags = tags_across_the_wrap();
    let mut iq = queue_across_the_wrap(&tags, |tag| ready_nop(tag.raw()));

    let selected = select(&mut iq, 2, &StoreBuffer::new(4), &Rob::new(8), usize::MAX, usize::MAX);

    let issued: Vec<RobTag> = selected.iter().map(|e| e.entry.rob_tag).collect();
    assert_eq!(issued, tags[..2]);
}

#[test]
fn the_issue_hold_reports_the_oldest_entry_across_the_tag_wrap() {
    let tags = tags_across_the_wrap();
    let oldest = tags[0];
    let mut iq = queue_across_the_wrap(&tags, |tag| {
        let waiting = OperandState::not_ready(PhysReg::from_index(5));
        let src1 = if tag == oldest { waiting } else { ready_operand(0) };
        IssueQueueEntry { src1, ..ready_nop(tag.raw()) }
    });
    let units = FuPool::new(&FuConfig::default());

    let selection = selection_under(
        &mut iq,
        &single_issue(&units, 0, false),
        &StoreBuffer::new(4),
        &Rob::new(8),
    );

    assert_eq!(selection.oldest, Some(IssueHold::Operands));
}

#[test]
fn the_queue_snapshot_lists_entries_oldest_first_across_the_tag_wrap() {
    let tags = tags_across_the_wrap();
    let iq = queue_across_the_wrap(&tags, |tag| ready_nop(tag.raw()));

    let snapshot: Vec<RobTag> = iq.queue_snapshot().iter().map(|e| e.rob_tag).collect();

    assert_eq!(snapshot, tags);
}

#[test]
fn dispatch_into_a_full_queue_leaves_the_rest_for_next_cycle() {
    let mut config = Config::minimal();
    config.pipeline.issue_queue_size = 2;
    let mut sys = SystemState::build(&config);
    let mut state = sys.core_ctx(0);
    let mut engine = O3Engine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
    let mut renamed = vec![make_entry(0), make_entry(1), make_entry(2)];

    engine.dispatch(&mut state, &mut renamed);

    assert_eq!(engine.issue_queue.available_slots(), 0);
    assert_eq!(renamed.iter().map(|entry| entry.rob_tag).collect::<Vec<_>>(), [RobTag::new(2)]);
}

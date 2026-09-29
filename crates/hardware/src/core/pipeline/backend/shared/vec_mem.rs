//! Vector memory instructions as micro-ops through the memory stages: one
//! `ExMem1Entry` per micro-op flows memory1 → memory2 → writeback, and the
//! instruction completes when its last micro-op has.
//!
//! A unit-stride access (plain, segment, fault-only-first, mask or
//! whole-register) moves its naturally aligned elements as spans, one
//! memory access per vector-memory-datapath window, as a vector unit's
//! load-store path does. Strided and indexed accesses, misaligned elements,
//! and a span that meets a fault, a trigger or a device go element by
//! element.

use std::collections::VecDeque;

use crate::common::error::{ExceptionStage, Trap};
use crate::core::exec::signals::{MemWidth, VectorOp};
use crate::core::pipeline::latches::{
    ExMem1Entry, Mem2WbEntry, MicroOpIdx, VecMemAccess, VecMemSpan, VecMemTarget,
};
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::vec_prf::VecPhysReg;
use crate::core::units::vpu::mem::VecMemAddrOp;
use crate::isa::rvv::{ElemIdx, Sew};

/// One micro-op of a vector memory instruction on its way through the
/// memory stages.
#[derive(Debug, Clone)]
pub struct VecMemMicroOp {
    /// The `ExMem1Entry` carrying the micro-op's address and metadata.
    pub entry: ExMem1Entry,
    /// This micro-op among its instruction's micro-ops.
    pub micro_op: MicroOpIdx,
    /// Bytes it reads or writes.
    pub bytes: usize,
    /// Whether this is a store (vs load).
    pub is_store: bool,
}

/// A vector memory instruction in flight: its micro-ops not yet written
/// back, and those not yet in the memory pipeline.
#[derive(Debug, Clone)]
pub struct VecMemInflight {
    /// ROB tag of the parent vector memory instruction.
    pub rob_tag: RobTag,
    /// Number of micro-ops still outstanding (not yet written back).
    pub remaining: usize,
    /// Physical destination registers for the LMUL group (for chaining wakeup).
    pub vd_phys: [VecPhysReg; 8],
    /// Number of destination registers in the LMUL group.
    pub vd_count: u8,
    /// Whether chaining wakeup has fired (first cache-line returned).
    pub wakeup_fired: bool,
    /// Micro-ops generated at issue but not yet pushed into the memory
    /// pipeline.
    pub pending_micro_ops: VecDeque<VecMemMicroOp>,
    /// For a fault-only-first load, the element whose fault trimmed `vl`:
    /// it and everything after it are tail elements.
    pub trimmed_at: Option<ElemIdx>,
    /// The lowest element found faulting so far, which the instruction
    /// reports once every micro-op has finished.
    pub fault: Option<ElementFault>,
}

/// A fault an element of a vector memory instruction met.
#[derive(Debug, Clone)]
pub struct ElementFault {
    /// The element.
    pub element: ElemIdx,
    /// Its trap.
    pub trap: Trap,
    /// The stage that raised it.
    pub stage: ExceptionStage,
}

/// How an instruction's element accesses go to memory.
#[derive(Debug, Clone)]
pub enum PlannedAccess {
    /// One element access on its own.
    Element(VecMemAddrOp),
    /// Element accesses moved as one span, each with its own micro-op.
    Span(Vec<(MicroOpIdx, VecMemAddrOp)>),
}

/// Convert an EEW byte count (1/2/4/8) to the corresponding `MemWidth`.
#[must_use]
pub const fn mem_width_from_eew_bytes(bytes: usize) -> MemWidth {
    match bytes {
        1 => MemWidth::Byte,
        2 => MemWidth::Half,
        8 => MemWidth::Double,
        _ => MemWidth::Word,
    }
}

/// True for the unit-stride accesses whose elements lie back to back in
/// memory, which the vector memory datapath moves a window at a time.
#[must_use]
pub const fn moves_in_spans(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VLoadUnit
            | VectorOp::VStoreUnit
            | VectorOp::VLoadFF
            | VectorOp::VLoadMask
            | VectorOp::VStoreMask
            | VectorOp::VLoadWholeReg
            | VectorOp::VStoreWholeReg
    )
}

/// Plans how `addresses`, an instruction's element accesses in address
/// order, go to memory.
///
/// With `in_spans`, consecutive naturally aligned accesses in one
/// `width`-byte window form a span; a window holding one access, and a
/// misaligned access, go alone. Each element access is the micro-op
/// numbered by its position, and each span a micro-op after them.
#[must_use]
pub fn plan_accesses(
    addresses: Vec<VecMemAddrOp>,
    in_spans: bool,
    width: usize,
) -> Vec<(MicroOpIdx, PlannedAccess)> {
    let mut planner = SpanPlanner { next_span: addresses.len(), planned: Vec::new() };
    let mut run: Vec<(MicroOpIdx, VecMemAddrOp)> = Vec::new();
    for (position, access) in addresses.into_iter().enumerate() {
        let micro_op = MicroOpIdx::new(position);
        if !in_spans || !is_naturally_aligned(&access) {
            planner.close(std::mem::take(&mut run));
            planner.planned.push((micro_op, PlannedAccess::Element(access)));
            continue;
        }
        if run.first().is_some_and(|(_, first)| window(first, width) != window(&access, width)) {
            planner.close(std::mem::take(&mut run));
        }
        run.push((micro_op, access));
    }
    planner.close(run);
    planner.planned
}

struct SpanPlanner {
    next_span: usize,
    planned: Vec<(MicroOpIdx, PlannedAccess)>,
}

impl SpanPlanner {
    /// Plans `run`, the accesses of one window: alone when it holds one,
    /// else as the next span.
    fn close(&mut self, mut run: Vec<(MicroOpIdx, VecMemAddrOp)>) {
        match run.len() {
            0 => {}
            1 => {
                let (micro_op, access) = run.remove(0);
                self.planned.push((micro_op, PlannedAccess::Element(access)));
            }
            _ => {
                let micro_op = MicroOpIdx::new(self.next_span);
                self.next_span += 1;
                self.planned.push((micro_op, PlannedAccess::Span(run)));
            }
        }
    }
}

const fn is_naturally_aligned(access: &VecMemAddrOp) -> bool {
    access.vaddr.val().is_multiple_of(access.eew.bytes() as u64)
}

const fn window(access: &VecMemAddrOp, width: usize) -> u64 {
    access.vaddr.val() / width as u64
}

/// The micro-ops of `parent`, one memory entry per planned access.
#[must_use]
pub fn micro_ops_for(
    parent: &ExMem1Entry,
    planned: Vec<(MicroOpIdx, PlannedAccess)>,
    is_store: bool,
) -> VecDeque<VecMemMicroOp> {
    planned
        .into_iter()
        .map(|(micro_op, access)| match access {
            PlannedAccess::Element(element) => {
                element_micro_op(parent, micro_op, &element, is_store)
            }
            PlannedAccess::Span(elements) => span_micro_op(parent, micro_op, elements, is_store),
        })
        .collect()
}

/// The micro-op of one element access of `parent`.
const fn element_micro_op(
    parent: &ExMem1Entry,
    micro_op: MicroOpIdx,
    element: &VecMemAddrOp,
    is_store: bool,
) -> VecMemMicroOp {
    let target = VecMemTarget::Element {
        elem_idx: element.elem_idx,
        eew: element.eew,
        vd_phys: element.vd_phys,
    };
    let access = VecMemAccess { micro_op, is_store, target };
    let width = mem_width_from_eew_bytes(element.eew.bytes());
    VecMemMicroOp {
        entry: memory_entry(parent, access, element.vaddr.val(), element.store_data, width),
        micro_op,
        bytes: element.eew.bytes(),
        is_store,
    }
}

/// The micro-op moving `elements` of `parent` as one span.
fn span_micro_op(
    parent: &ExMem1Entry,
    micro_op: MicroOpIdx,
    elements: Vec<(MicroOpIdx, VecMemAddrOp)>,
    is_store: bool,
) -> VecMemMicroOp {
    let span = VecMemSpan { elements, data: None };
    let (vaddr, bytes) = (span.vaddr().val(), span.bytes());
    let access = VecMemAccess { micro_op, is_store, target: VecMemTarget::Span(Box::new(span)) };
    VecMemMicroOp {
        entry: memory_entry(parent, access, vaddr, 0, MemWidth::Nop),
        micro_op,
        bytes,
        is_store,
    }
}

/// `parent`'s memory entry for one access at `vaddr`.
const fn memory_entry(
    parent: &ExMem1Entry,
    access: VecMemAccess,
    vaddr: u64,
    store_data: u64,
    width: MemWidth,
) -> ExMem1Entry {
    let mut ctrl = parent.ctrl;
    ctrl.mem_read = !access.is_store;
    ctrl.mem_write = access.is_store;
    ctrl.width = width;
    ExMem1Entry {
        rob_tag: parent.rob_tag,
        pc: parent.pc,
        inst: parent.inst,
        inst_size: parent.inst_size,
        rd: parent.rd,
        rd_phys: parent.rd_phys,
        alu: vaddr,
        store_data,
        ctrl,
        trap: None,
        exception_stage: None,
        fp_flags: 0,
        sfence_vma: None,
        vec_mem: Some(access),
    }
}

/// Takes apart the span `entry` carries into its element micro-ops.
///
/// They go first among its instruction's pending micro-ops, for a fault, a
/// trigger or a device to meet element by element. Returns the span's
/// micro-op, whose load-queue entry the elements' replace.
pub fn expand_span(entry: &ExMem1Entry, inflight: &mut [VecMemInflight]) -> Option<MicroOpIdx> {
    let access = entry.vec_mem.as_ref()?;
    let VecMemTarget::Span(span) = &access.target else { return None };
    let parent = inflight.iter_mut().find(|m| m.rob_tag == entry.rob_tag)?;
    let elements: Vec<VecMemMicroOp> = span
        .elements
        .iter()
        .map(|(micro_op, element)| element_micro_op(entry, *micro_op, element, access.is_store))
        .collect();
    parent.remaining += elements.len().saturating_sub(1);
    for element in elements.into_iter().rev() {
        parent.pending_micro_ops.push_front(element);
    }
    Some(access.micro_op)
}

/// An element value a finished load micro-op writes to its register.
#[derive(Debug, Clone, Copy)]
pub struct ElementValue {
    /// Element index within the vector register group.
    pub elem_idx: ElemIdx,
    /// Its width.
    pub eew: Sew,
    /// The physical register it goes to.
    pub vd_phys: VecPhysReg,
    /// The value, zero-extended.
    pub value: u64,
}

/// What a micro-op's writeback means for its instruction.
#[derive(Debug, Clone)]
pub struct AccessRetired {
    /// The element values that go to their destination registers.
    pub writes: Vec<ElementValue>,
    /// This was the instruction's last micro-op.
    pub completed: bool,
}

/// Retires one micro-op that reached writeback.
///
/// When its last micro-op retires the instruction completes, or faults at
/// the lowest element that faulted, with `vstart` there: its micro-ops run
/// in any order, so a fault waits for the elements below it. A faulting
/// element of a fault-only-first load past its first trims `vl` instead,
/// and the elements from it on become tail.
pub fn retire_access(
    wb: &Mem2WbEntry,
    access: &VecMemAccess,
    inflight: &mut [VecMemInflight],
    rob: &mut Rob,
) -> AccessRetired {
    let Some(parent) = inflight.iter_mut().find(|m| m.rob_tag == wb.rob_tag) else {
        return AccessRetired { writes: Vec::new(), completed: false };
    };
    let values = element_values(wb, access);
    if let (Some(trap), Some(first)) = (&wb.trap, values.first()) {
        let trims = wb.ctrl.vec_op == VectorOp::VLoadFF && first.elem_idx.as_usize() > 0;
        if trims {
            if parent.trimmed_at.is_none_or(|at| first.elem_idx < at) {
                parent.trimmed_at = Some(first.elem_idx);
                rob.set_vl_trim(wb.rob_tag, first.elem_idx.as_usize() as u64);
            }
        } else if parent.fault.as_ref().is_none_or(|fault| first.elem_idx < fault.element) {
            let stage = wb.exception_stage.unwrap_or(ExceptionStage::Memory);
            parent.fault =
                Some(ElementFault { element: first.elem_idx, trap: trap.clone(), stage });
        }
    }
    let writes = if wb.trap.is_some() || access.is_store {
        Vec::new()
    } else {
        values.into_iter().filter(|v| parent.trimmed_at.is_none_or(|at| v.elem_idx < at)).collect()
    };
    parent.remaining = parent.remaining.saturating_sub(1);
    let completed = parent.remaining == 0;
    if completed {
        match parent.fault.take() {
            Some(fault) => {
                let element = fault.element.as_usize() as u64;
                rob.fault_element(wb.rob_tag, fault.trap, fault.stage, element);
            }
            None => rob.complete(wb.rob_tag, 0),
        }
    }
    AccessRetired { writes, completed }
}

/// The elements a micro-op carries, with the values a load read for them.
fn element_values(wb: &Mem2WbEntry, access: &VecMemAccess) -> Vec<ElementValue> {
    match &access.target {
        VecMemTarget::Element { elem_idx, eew, vd_phys } => vec![ElementValue {
            elem_idx: *elem_idx,
            eew: *eew,
            vd_phys: *vd_phys,
            value: wb.load_data,
        }],
        VecMemTarget::Span(span) => span
            .elements
            .iter()
            .map(|(_, element)| {
                let offset = span.offset_of(element);
                let bytes = span
                    .data
                    .as_deref()
                    .and_then(|data| data.get(offset..offset + element.eew.bytes()));
                let value = bytes.map_or(0, |bytes| {
                    bytes.iter().rev().fold(0u64, |value, &byte| (value << 8) | u64::from(byte))
                });
                ElementValue {
                    elem_idx: element.elem_idx,
                    eew: element.eew,
                    vd_phys: element.vd_phys,
                    value,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::VirtAddr;

    fn access(elem: usize, vaddr: u64, eew: Sew) -> VecMemAddrOp {
        VecMemAddrOp {
            vaddr: VirtAddr::new(vaddr),
            store_data: 0,
            elem_idx: ElemIdx::new(elem),
            eew,
            vd_phys: VecPhysReg::ZERO,
        }
    }

    /// Each planned micro-op as `(index, [element micro-ops])`, a lone
    /// element as its own index.
    fn shape(planned: &[(MicroOpIdx, PlannedAccess)]) -> Vec<(MicroOpIdx, Vec<MicroOpIdx>)> {
        planned
            .iter()
            .map(|(micro_op, access)| match access {
                PlannedAccess::Element(_) => (*micro_op, vec![*micro_op]),
                PlannedAccess::Span(elements) => {
                    (*micro_op, elements.iter().map(|(m, _)| *m).collect())
                }
            })
            .collect()
    }

    const fn m(index: usize) -> MicroOpIdx {
        MicroOpIdx::new(index)
    }

    #[test]
    fn the_fields_of_one_segment_element_are_different_micro_ops() {
        let parent = ExMem1Entry::default();
        let addresses = vec![
            access(0, 0x1000, Sew::E32),
            access(0, 0x1004, Sew::E32),
            access(1, 0x1008, Sew::E32),
        ];

        let micro_ops: Vec<MicroOpIdx> =
            micro_ops_for(&parent, plan_accesses(addresses, false, 32), false)
                .iter()
                .map(|op| op.micro_op)
                .collect();

        assert_eq!(micro_ops, vec![m(0), m(1), m(2)]);
    }

    #[test]
    fn aligned_elements_group_by_window() {
        let addresses: Vec<_> =
            (0..12).map(|i| access(i, 0x1000 + 8 * i as u64, Sew::E64)).collect();

        let planned = plan_accesses(addresses, true, 32);

        assert_eq!(
            shape(&planned),
            vec![
                (m(12), vec![m(0), m(1), m(2), m(3)]),
                (m(13), vec![m(4), m(5), m(6), m(7)]),
                (m(14), vec![m(8), m(9), m(10), m(11)]),
            ]
        );
    }

    #[test]
    fn a_misaligned_base_leaves_its_elements_alone() {
        let addresses: Vec<_> =
            (0..3).map(|i| access(i, 0x1004 + 8 * i as u64, Sew::E64)).collect();

        let planned = plan_accesses(addresses, true, 32);

        assert_eq!(
            shape(&planned),
            vec![(m(0), vec![m(0)]), (m(1), vec![m(1)]), (m(2), vec![m(2)])]
        );
    }

    #[test]
    fn a_window_with_one_element_sends_it_alone() {
        let addresses = vec![
            access(0, 0x1018, Sew::E64),
            access(1, 0x1020, Sew::E64),
            access(2, 0x1028, Sew::E64),
        ];

        let planned = plan_accesses(addresses, true, 32);

        assert_eq!(shape(&planned), vec![(m(0), vec![m(0)]), (m(3), vec![m(1), m(2)])]);
    }

    #[test]
    fn masked_off_elements_leave_holes_inside_a_span() {
        let addresses = vec![access(0, 0x1000, Sew::E32), access(3, 0x100C, Sew::E32)];

        let planned = plan_accesses(addresses, true, 16);

        let PlannedAccess::Span(elements) = &planned[0].1 else { panic!("expected a span") };
        let span = VecMemSpan { elements: elements.clone(), data: None };
        assert_eq!((span.vaddr().val(), span.bytes()), (0x1000, 16));
    }

    #[test]
    fn strided_accesses_stay_element_by_element() {
        let addresses: Vec<_> =
            (0..4).map(|i| access(i, 0x1000 + 4 * i as u64, Sew::E32)).collect();

        let planned = plan_accesses(addresses, false, 32);

        assert_eq!(planned.len(), 4);
    }
}

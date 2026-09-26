//! Vector memory instructions as element micro-ops through the memory
//! stages: one `ExMem1Entry` per element flows memory1 → memory2 →
//! writeback, and the instruction completes when its last element has.

use std::collections::VecDeque;

use crate::common::error::ExceptionStage;
use crate::core::pipeline::latches::{ExMem1Entry, Mem2WbEntry, VecMemElement};
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::signals::{MemWidth, VectorOp};
use crate::core::units::vpu::mem::VecMemAddrOp;
use crate::core::units::vpu::types::{ElemIdx, Sew, VecPhysReg};

/// One element of a vector memory instruction on its way through the
/// memory stages.
#[derive(Debug, Clone)]
pub struct VecMemMicroOp {
    /// The `ExMem1Entry` carrying the element's virtual address and metadata.
    pub entry: ExMem1Entry,
    /// Element index within the vector register (for writeback targeting).
    pub elem_idx: ElemIdx,
    /// Effective element width for this access.
    pub eew: Sew,
    /// Destination physical vector register for this element's data.
    pub vd_phys: VecPhysReg,
    /// Whether this is a store (vs load).
    pub is_store: bool,
}

/// A vector memory instruction in flight: its elements not yet written
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

/// The micro-ops of `parent`, one memory entry per element address.
#[must_use]
pub fn micro_ops_for(
    parent: &ExMem1Entry,
    addresses: Vec<VecMemAddrOp>,
    is_store: bool,
) -> VecDeque<VecMemMicroOp> {
    addresses
        .into_iter()
        .map(|mop| {
            let mut ctrl = parent.ctrl;
            ctrl.mem_read = !is_store;
            ctrl.mem_write = is_store;
            ctrl.width = mem_width_from_eew_bytes(mop.eew.bytes());
            let element = VecMemElement {
                elem_idx: mop.elem_idx,
                eew: mop.eew,
                vd_phys: mop.vd_phys,
                is_store,
            };
            VecMemMicroOp {
                entry: ExMem1Entry {
                    rob_tag: parent.rob_tag,
                    pc: parent.pc,
                    inst: parent.inst,
                    inst_size: parent.inst_size,
                    rd: parent.rd,
                    rd_phys: parent.rd_phys,
                    alu: mop.vaddr.val(),
                    store_data: mop.store_data,
                    ctrl,
                    trap: None,
                    exception_stage: None,
                    fp_flags: 0,
                    sfence_vma: None,
                    vec_mem: Some(element),
                },
                elem_idx: mop.elem_idx,
                eew: mop.eew,
                vd_phys: mop.vd_phys,
                is_store,
            }
        })
        .collect()
}

/// What an element's writeback means for its instruction.
#[derive(Debug, Clone, Copy)]
pub struct ElementRetired {
    /// The element's data goes to its destination register.
    pub write_data: bool,
    /// This was the instruction's last element.
    pub completed: bool,
}

/// Retires one element that reached writeback.
///
/// A faulting element faults the instruction with `vstart` at that
/// element, except in a fault-only-first load past its first element,
/// where it trims `vl` instead and the elements from it on become tail.
/// The instruction completes when its last element retires.
pub fn retire_element(
    wb: &Mem2WbEntry,
    element: &VecMemElement,
    inflight: &mut [VecMemInflight],
    rob: &mut Rob,
) -> ElementRetired {
    let Some(parent) = inflight.iter_mut().find(|m| m.rob_tag == wb.rob_tag) else {
        return ElementRetired { write_data: false, completed: false };
    };
    if let Some(trap) = &wb.trap {
        let trims = wb.ctrl.vec_op == VectorOp::VLoadFF && element.elem_idx.as_usize() > 0;
        if trims {
            if parent.trimmed_at.is_none_or(|at| element.elem_idx < at) {
                parent.trimmed_at = Some(element.elem_idx);
                rob.set_vl_trim(wb.rob_tag, element.elem_idx.as_usize() as u64);
            }
        } else {
            rob.fault(
                wb.rob_tag,
                trap.clone(),
                wb.exception_stage.unwrap_or(ExceptionStage::Memory),
            );
            rob.set_fault_vstart(wb.rob_tag, element.elem_idx.as_usize() as u64);
        }
    }
    let is_tail = parent.trimmed_at.is_some_and(|at| element.elem_idx >= at);
    let write_data = wb.trap.is_none() && !element.is_store && !is_tail;
    parent.remaining = parent.remaining.saturating_sub(1);
    let completed = parent.remaining == 0;
    if completed {
        rob.complete(wb.rob_tag, 0);
    }
    ElementRetired { write_data, completed }
}

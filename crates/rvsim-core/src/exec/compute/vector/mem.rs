//! Per-element address generation for vector loads and stores.
//!
//! Every variant (unit-stride, strided, indexed, mask, whole-register,
//! fault-only-first) becomes a list of element accesses that the backends
//! run through the memory stages as micro-ops.

use crate::common::VirtAddr;
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::exec::signals::ControlSignals;
use crate::isa::op::{VecLoadOp, VecStoreOp, VectorOp};
use crate::isa::privileged::Trap;
use crate::isa::rvv::{ElemIdx, Emul, Nf, Sew, VRegIdx, VectorConfig, VtypeFields};

/// Returns `(data_emul_regs, idx_emul_regs)` for a vec memory op.
///
/// `data_emul_regs` is the destination (or stored value) register-group size
/// in registers; `idx_emul_regs` is the index vector size in registers for
/// indexed loads/stores (0 otherwise). Each is at least 1, mirroring the
/// spec's `max(1, EEW × LMUL / SEW)` floor. A whole-register access counts
/// one register per field; [`vec_mem_dst_count`] gives all of them.
#[must_use]
pub fn vec_mem_emul_regs(
    op: VectorOp,
    eew: Sew,
    sew: Sew,
    lmul: crate::isa::rvv::Vlmul,
) -> (u8, u8) {
    let (lnum, lden) = lmul.as_fraction();
    let lmul_regs = if lnum >= lden { (lnum / lden) as u8 } else { 1 };
    match op {
        VectorOp::Load(VecLoadOp::IndexedOrdered | VecLoadOp::IndexedUnordered)
        | VectorOp::Store(VecStoreOp::IndexedOrdered | VecStoreOp::IndexedUnordered) => {
            // Indexed: data EMUL = LMUL, index EMUL = (idx_EEW × LMUL) / SEW.
            let idx_num = eew.bits() * lnum;
            let idx_den = sew.bits() * lden;
            let idx = if idx_num >= idx_den { ((idx_num / idx_den) as u8).max(1) } else { 1 };
            (lmul_regs, idx)
        }
        VectorOp::Load(VecLoadOp::Unit | VecLoadOp::FaultOnlyFirst | VecLoadOp::Strided)
        | VectorOp::Store(VecStoreOp::Unit | VecStoreOp::Strided) => {
            // Unit/strided: data EMUL = (EEW × LMUL) / SEW.
            let num = eew.bits() * lnum;
            let den = sew.bits() * lden;
            let emul = if num >= den { ((num / den) as u8).max(1) } else { 1 };
            (emul, 0)
        }
        // Neither depends on LMUL: a mask is one register, and a
        // whole-register access moves its NFIELDS registers one by one.
        VectorOp::Load(VecLoadOp::Mask | VecLoadOp::WholeRegister)
        | VectorOp::Store(VecStoreOp::Mask | VecStoreOp::WholeRegister) => (1, 0),
        _ => (lmul_regs, 0),
    }
}

/// Returns the total destination register-group size for a vec mem op,
/// already multiplied by the segment count `nf+1`.
#[must_use]
pub fn vec_mem_dst_count(
    op: VectorOp,
    eew: Sew,
    sew: Sew,
    lmul: crate::isa::rvv::Vlmul,
    nf_field: u8,
) -> u8 {
    let (data_emul, _) = vec_mem_emul_regs(op, eew, sew, lmul);
    let nf = (u16::from(nf_field) + 1).min(8) as u8;
    match op {
        VectorOp::Load(VecLoadOp::Mask) | VectorOp::Store(VecStoreOp::Mask) => 1,
        VectorOp::Load(VecLoadOp::WholeRegister) | VectorOp::Store(VecStoreOp::WholeRegister) => {
            (nf_field + 1).min(8)
        }
        _ => data_emul.saturating_mul(nf).min(8),
    }
}

/// Reject vector memory ops that violate the §10.1.4 register-group rules.
///
/// Encodings producing `EMUL > 8`, `EMUL < 1/8`, or a destination that is
/// not aligned to `EMUL × NF` are reserved.
///
/// # Errors
///
/// Returns `Err(Trap::IllegalInstruction)` when the op's EMUL exceeds 8,
/// the destination overflows past `v31`, or the destination is misaligned.
#[allow(clippy::missing_const_for_fn)]
pub fn check_vec_mem_emul(
    inst: u32,
    ctrl: &ControlSignals,
    vtype: VtypeFields,
) -> Result<(), Trap> {
    if vtype.vill {
        return Ok(());
    }
    let eew = ctrl.vec_eew;
    let sew = vtype.vsew;
    let lmul = vtype.vlmul;
    let vd = ctrl.vd.as_u8() as usize;
    let Some(op) = ctrl.vec_op else {
        return Ok(());
    };
    let illegal = match op {
        VectorOp::Load(VecLoadOp::Unit | VecLoadOp::FaultOnlyFirst | VecLoadOp::Strided)
        | VectorOp::Store(VecStoreOp::Unit | VecStoreOp::Strided) => {
            let nf = (ctrl.vec_nf as usize).saturating_add(1);
            let (lnum, lden) = lmul.as_fraction();
            let emul_num = eew.bits() * lnum;
            let emul_den = sew.bits() * lden;
            let emul = if emul_num >= emul_den { emul_num / emul_den } else { 0 };
            let total = emul.saturating_mul(nf);
            emul > 8 || total > 8 || vd + total > 32 || (emul > 0 && !vd.is_multiple_of(emul))
        }
        VectorOp::Load(VecLoadOp::IndexedOrdered | VecLoadOp::IndexedUnordered)
        | VectorOp::Store(VecStoreOp::IndexedOrdered | VecStoreOp::IndexedUnordered) => {
            // Index EMUL = (idx_EEW * LMUL) / SEW; data EMUL = LMUL.
            let nf = (ctrl.vec_nf as usize).saturating_add(1);
            let (lnum, lden) = lmul.as_fraction();
            let idx_num = eew.bits() * lnum;
            let idx_den = sew.bits() * lden;
            let idx_emul = if idx_num >= idx_den { idx_num / idx_den } else { 0 };
            let data_emul = if lnum >= lden { lnum / lden } else { 0 };
            let total = data_emul.saturating_mul(nf);
            idx_emul > 8
                || total > 8
                || vd + total > 32
                || (data_emul > 0 && !vd.is_multiple_of(data_emul))
        }
        // Mask and whole-register accesses are constrained at the encoding
        // level (NF = 0/1/3/7) and never exceed 8 registers.
        _ => false,
    };
    if illegal {
        return Err(Trap::IllegalInstruction(inst));
    }
    Ok(())
}

/// One element (or segment field) a vector memory instruction accesses.
#[derive(Debug, Clone)]
pub struct ElementAccess {
    /// Virtual address for this element access.
    pub vaddr: VirtAddr,
    /// The element's data for a store, 0 for a load.
    pub store_data: u64,
    /// Element index within the destination vector register.
    pub elem_idx: ElemIdx,
    /// Effective element width for this access.
    pub eew: Sew,
    /// Which register of the destination group the element lands in,
    /// counting from `vd`.
    pub dest_slot: usize,
}

/// Whether `op` is a vector load.
pub const fn is_vec_load(op: Option<VectorOp>) -> bool {
    matches!(op, Some(VectorOp::Load(_)))
}

/// Whether `op` is a vector store.
pub const fn is_vec_store(op: Option<VectorOp>) -> bool {
    matches!(op, Some(VectorOp::Store(_)))
}

/// The register of the destination group element `elem` of field `seg`
/// lands in: `seg * emul_regs + elem / elements_per_reg`.
const fn dest_slot(elem: usize, seg: usize, emul_regs: usize, elements_per_reg: usize) -> usize {
    let reg_in_seg = match elem.checked_div(elements_per_reg) {
        Some(reg) => reg,
        None => 0,
    };
    seg * emul_regs + reg_in_seg
}

/// The element accesses a vector memory instruction makes, reading store
/// data and index values from `vrf`. Performs no memory access.
#[must_use]
pub fn element_accesses<V: VectorRegFile>(
    vrf: &V,
    base_addr: u64,
    stride: i64,
    ctrl: &ControlSignals,
    config: VectorConfig,
) -> Vec<ElementAccess> {
    let Some(vec_op) = ctrl.vec_op else {
        return Vec::new();
    };
    let vtype = config.fields();
    let (vl, vstart) = (config.vl as usize, config.vstart as usize);
    if vtype.vill && vec_op.depends_on_vtype() {
        return Vec::new();
    }

    let is_store = matches!(vec_op, VectorOp::Store(_));
    let eew = ctrl.vec_eew;
    let vm = ctrl.vm;
    let vd = ctrl.vd;
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let vlen = vrf.vlen();

    match vec_op {
        VectorOp::Load(VecLoadOp::Unit | VecLoadOp::FaultOnlyFirst)
        | VectorOp::Store(VecStoreOp::Unit) => {
            let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
            let eew_bytes = eew.bytes() as u64;
            let elements_per_reg = vlen.bits() / (eew.bytes() * 8);
            let emul_regs = emul.regs() as usize;
            let mut ops = Vec::with_capacity(vl * nf.fields_usize());
            for i in vstart..vl {
                if !is_element_active_vrf(vrf, i, vm) {
                    continue;
                }
                for seg in 0..nf.fields_usize() {
                    let addr = base_addr.wrapping_add(
                        ((i * nf.fields_usize() + seg) as u64).wrapping_mul(eew_bytes),
                    );
                    let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
                    let store_data =
                        if is_store { vrf.read_element(dest, ElemIdx::new(i), eew) } else { 0 };
                    let slot = dest_slot(i, seg, emul_regs, elements_per_reg);
                    ops.push(ElementAccess {
                        vaddr: VirtAddr::new(addr),
                        store_data,
                        elem_idx: ElemIdx::new(i),
                        eew,
                        dest_slot: slot,
                    });
                }
            }
            ops
        }
        VectorOp::Load(VecLoadOp::Strided) | VectorOp::Store(VecStoreOp::Strided) => {
            let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
            let eew_bytes = eew.bytes() as u64;
            let elements_per_reg = vlen.bits() / (eew.bytes() * 8);
            let emul_regs = emul.regs() as usize;
            let mut ops = Vec::with_capacity(vl * nf.fields_usize());
            for i in vstart..vl {
                if !is_element_active_vrf(vrf, i, vm) {
                    continue;
                }
                let elem_base =
                    base_addr.wrapping_add((i as i64).wrapping_mul(stride).cast_unsigned());
                for seg in 0..nf.fields_usize() {
                    let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
                    let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
                    let store_data =
                        if is_store { vrf.read_element(dest, ElemIdx::new(i), eew) } else { 0 };
                    let slot = dest_slot(i, seg, emul_regs, elements_per_reg);
                    ops.push(ElementAccess {
                        vaddr: VirtAddr::new(addr),
                        store_data,
                        elem_idx: ElemIdx::new(i),
                        eew,
                        dest_slot: slot,
                    });
                }
            }
            ops
        }
        VectorOp::Load(VecLoadOp::IndexedOrdered | VecLoadOp::IndexedUnordered)
        | VectorOp::Store(VecStoreOp::IndexedOrdered | VecStoreOp::IndexedUnordered) => {
            let vs2 = ctrl.vs2;
            let data_sew = vtype.vsew;
            let idx_eew = eew;
            let data_bytes = data_sew.bytes() as u64;
            let data_emul = Emul::compute(data_sew, data_sew, vtype.vlmul);
            let elements_per_reg = vlen.bits() / (data_sew.bytes() * 8);
            let emul_regs = data_emul.regs() as usize;
            let mut ops = Vec::with_capacity(vl * nf.fields_usize());
            for i in vstart..vl {
                if !is_element_active_vrf(vrf, i, vm) {
                    continue;
                }
                let offset = vrf.read_element(vs2, ElemIdx::new(i), idx_eew);
                let elem_base = base_addr.wrapping_add(offset);
                for seg in 0..nf.fields_usize() {
                    let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
                    let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * data_emul.regs());
                    let store_data = if is_store {
                        vrf.read_element(dest, ElemIdx::new(i), data_sew)
                    } else {
                        0
                    };
                    let slot = dest_slot(i, seg, emul_regs, elements_per_reg);
                    ops.push(ElementAccess {
                        vaddr: VirtAddr::new(addr),
                        store_data,
                        elem_idx: ElemIdx::new(i),
                        eew: data_sew,
                        dest_slot: slot,
                    });
                }
            }
            ops
        }
        VectorOp::Load(VecLoadOp::Mask) | VectorOp::Store(VecStoreOp::Mask) => {
            let num_bytes = vl.div_ceil(8);
            let elements_per_reg = vlen.bits() / 8; // E8
            let mut ops = Vec::with_capacity(num_bytes);
            for i in 0..num_bytes {
                let addr = base_addr.wrapping_add(i as u64);
                let store_data =
                    if is_store { vrf.read_element(vd, ElemIdx::new(i), Sew::E8) } else { 0 };
                let slot = dest_slot(i, 0, 1, elements_per_reg);
                ops.push(ElementAccess {
                    vaddr: VirtAddr::new(addr),
                    store_data,
                    elem_idx: ElemIdx::new(i),
                    eew: Sew::E8,
                    dest_slot: slot,
                });
            }
            ops
        }
        VectorOp::Load(VecLoadOp::WholeRegister) | VectorOp::Store(VecStoreOp::WholeRegister) => {
            let nreg = (ctrl.vec_nf as usize) + 1;
            let vlen_bytes = vlen.bytes();
            let total_bytes = nreg * vlen_bytes;
            let mut ops = Vec::with_capacity(total_bytes);
            for i in 0..total_bytes {
                let addr = base_addr.wrapping_add(i as u64);
                let reg_offset = i / vlen_bytes;
                let byte_offset = i % vlen_bytes;
                let src = VRegIdx::new(vd.as_u8() + reg_offset as u8);
                let store_data = if is_store {
                    vrf.read_element(src, ElemIdx::new(byte_offset), Sew::E8)
                } else {
                    0
                };
                ops.push(ElementAccess {
                    vaddr: VirtAddr::new(addr),
                    store_data,
                    elem_idx: ElemIdx::new(byte_offset),
                    eew: Sew::E8,
                    dest_slot: reg_offset,
                });
            }
            ops
        }
        _ => Vec::new(),
    }
}

/// Check if element `i` is active under the mask using a generic `VectorRegFile`.
fn is_element_active_vrf<V: VectorRegFile>(vrf: &V, i: usize, vm: bool) -> bool {
    if vm {
        return true;
    }
    vrf.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::isa::op::{IntOp, VecAluOp};

    #[test]
    fn is_vec_load_accepts_every_load_kind_and_nothing_else() {
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::Unit))));
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::FaultOnlyFirst))));
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::Strided))));
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::IndexedOrdered))));
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::IndexedUnordered))));
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::Mask))));
        assert!(is_vec_load(Some(VectorOp::Load(VecLoadOp::WholeRegister))));
        assert!(!is_vec_load(Some(VectorOp::Store(VecStoreOp::Unit))));
        assert!(!is_vec_load(Some(VectorOp::Alu(VecAluOp::Int(IntOp::Add)))));
        assert!(!is_vec_load(None));
    }

    #[test]
    fn is_vec_store_accepts_every_store_kind_and_rejects_others() {
        assert!(is_vec_store(Some(VectorOp::Store(VecStoreOp::Unit))));
        assert!(is_vec_store(Some(VectorOp::Store(VecStoreOp::Strided))));
        assert!(is_vec_store(Some(VectorOp::Store(VecStoreOp::IndexedOrdered))));
        assert!(is_vec_store(Some(VectorOp::Store(VecStoreOp::IndexedUnordered))));
        assert!(is_vec_store(Some(VectorOp::Store(VecStoreOp::Mask))));
        assert!(is_vec_store(Some(VectorOp::Store(VecStoreOp::WholeRegister))));
        assert!(!is_vec_store(Some(VectorOp::Load(VecLoadOp::Unit))));
        assert!(!is_vec_store(Some(VectorOp::Alu(VecAluOp::Int(IntOp::Add)))));
    }
}

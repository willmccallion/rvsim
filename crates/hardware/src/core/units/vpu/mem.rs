//! Vector memory operations.
//!
//! Implements per-element address computation and memory access for all vector
//! load/store variants: unit-stride, strided, indexed, mask, whole-register,
//! and fault-only-first. All accesses go through the CPU's address translation
//! and bus interface.

use crate::core::pipeline::signals::MemWidth;
use crate::common::{AccessType, Trap, VirtAddr};
use crate::sim::CoreCtx;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::signals::{ControlSignals, VectorOp};
use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::vpu::types::{
    ElemIdx, Emul, Nf, Sew, VRegIdx, VecPhysReg, VtypeFields, parse_vtype,
};

/// Returns `(data_emul_regs, idx_emul_regs)` for a vec memory op.
///
/// `data_emul_regs` is the destination (or stored value) register-group size
/// in registers; `idx_emul_regs` is the index vector size in registers for
/// indexed loads/stores (0 otherwise). Each is at least 1, mirroring the
/// spec's `max(1, EEW × LMUL / SEW)` floor.
#[must_use]
pub fn vec_mem_emul_regs(
    op: VectorOp,
    eew: Sew,
    sew: Sew,
    lmul: crate::core::units::vpu::types::Vlmul,
) -> (u8, u8) {
    let (lnum, lden) = lmul.as_fraction();
    let lmul_regs = if lnum >= lden { (lnum / lden) as u8 } else { 1 };
    match op {
        VectorOp::VLoadIndexOrd
        | VectorOp::VLoadIndexUnord
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
            // Indexed: data EMUL = LMUL, index EMUL = (idx_EEW × LMUL) / SEW.
            let idx_num = eew.bits() * lnum;
            let idx_den = sew.bits() * lden;
            let idx = if idx_num >= idx_den { ((idx_num / idx_den) as u8).max(1) } else { 1 };
            (lmul_regs, idx)
        }
        VectorOp::VLoadUnit
        | VectorOp::VLoadFF
        | VectorOp::VStoreUnit
        | VectorOp::VLoadStride
        | VectorOp::VStoreStride => {
            // Unit/strided: data EMUL = (EEW × LMUL) / SEW.
            let num = eew.bits() * lnum;
            let den = sew.bits() * lden;
            let emul = if num >= den { ((num / den) as u8).max(1) } else { 1 };
            (emul, 0)
        }
        // Mask and whole-reg ops have fixed group sizes encoded elsewhere.
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
    lmul: crate::core::units::vpu::types::Vlmul,
    nf_field: u8,
) -> u8 {
    let (data_emul, _) = vec_mem_emul_regs(op, eew, sew, lmul);
    let nf = (nf_field as u16 + 1).min(8) as u8;
    match op {
        VectorOp::VLoadMask | VectorOp::VStoreMask => 1,
        VectorOp::VLoadWholeReg | VectorOp::VStoreWholeReg => (nf_field + 1).min(8),
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
    op: VectorOp,
    ctrl: &ControlSignals,
    vtype: &VtypeFields,
) -> Result<(), Trap> {
    if vtype.vill {
        return Ok(());
    }
    let eew = ctrl.vec_eew;
    let sew = vtype.vsew;
    let lmul = vtype.vlmul;
    let vd = ctrl.vd.as_u8() as usize;
    let illegal = match op {
        VectorOp::VLoadUnit
        | VectorOp::VStoreUnit
        | VectorOp::VLoadFF
        | VectorOp::VLoadStride
        | VectorOp::VStoreStride => {
            let nf = (ctrl.vec_nf as usize).saturating_add(1);
            let (lnum, lden) = lmul.as_fraction();
            let emul_num = eew.bits() * lnum;
            let emul_den = sew.bits() * lden;
            let emul = if emul_num >= emul_den { emul_num / emul_den } else { 0 };
            let total = emul.saturating_mul(nf);
            emul > 8 || total > 8 || vd + total > 32 || (emul > 0 && !vd.is_multiple_of(emul))
        }
        VectorOp::VLoadIndexOrd
        | VectorOp::VLoadIndexUnord
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
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

/// A single element address micro-op generated for the O3 vector memory pipeline.
///
/// Each micro-op represents one element (or segment field) that will flow
/// independently through Memory1 → Memory2 → Writeback.
#[derive(Debug, Clone)]
pub struct VecMemAddrOp {
    /// Virtual address for this element access.
    pub vaddr: VirtAddr,
    /// Store data (0 for loads — data was already written by functional exec).
    pub store_data: u64,
    /// Element index within the destination vector register.
    pub elem_idx: ElemIdx,
    /// Effective element width for this access.
    pub eew: Sew,
    /// Destination physical vector register for this element.
    pub vd_phys: VecPhysReg,
}

/// Execute a vector load operation. Returns 0 (no scalar result).
///
/// Reads elements from memory into the vector register file. Handles all
/// load variants: unit-stride, strided, indexed, mask, whole-register,
/// and fault-only-first.
///
/// # Errors
///
/// Returns a `Trap` if any element access causes an address translation
/// fault or access fault (except for fault-only-first loads where only
/// element 0 faults propagate).
pub fn execute_vec_load(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let vtype = parse_vtype(state.hart.csrs.vtype);
    check_vec_mem_emul(id.inst, id.ctrl.vec_op, &id.ctrl, &vtype)?;
    let eew = id.ctrl.vec_eew;
    let vd = id.ctrl.vd;
    let base_addr = id.rv1; // rs1 holds the base address

    match id.ctrl.vec_op {
        VectorOp::VLoadUnit => exec_unit_stride_load(state, base_addr, vd, eew, id),
        VectorOp::VLoadFF => exec_fault_first_load(state, base_addr, vd, eew, id),
        VectorOp::VLoadStride => {
            let stride = id.rv2 as i64;
            exec_strided_load(state, base_addr, stride, vd, eew, id)
        }
        VectorOp::VLoadIndexOrd | VectorOp::VLoadIndexUnord => {
            exec_indexed_load(state, base_addr, vd, eew, id)
        }
        VectorOp::VLoadMask => exec_mask_load(state, base_addr, vd, id),
        VectorOp::VLoadWholeReg => exec_whole_reg_load(state, base_addr, vd, eew, id),
        _ => Ok(0),
    }
}

/// Execute a vector store operation. Returns 0 (no scalar result).
///
/// Writes elements from the vector register file to memory. Handles all
/// store variants: unit-stride, strided, indexed, mask, and whole-register.
///
/// # Errors
///
/// Returns a `Trap` if any element access causes an address translation
/// fault or access fault.
pub fn execute_vec_store(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let vtype = parse_vtype(state.hart.csrs.vtype);
    check_vec_mem_emul(id.inst, id.ctrl.vec_op, &id.ctrl, &vtype)?;
    let eew = id.ctrl.vec_eew;
    let vs3 = id.ctrl.vd; // vd field encodes vs3 (store data source) for stores
    let base_addr = id.rv1;

    match id.ctrl.vec_op {
        VectorOp::VStoreUnit => exec_unit_stride_store(state, base_addr, vs3, eew, id),
        VectorOp::VStoreStride => {
            let stride = id.rv2 as i64;
            exec_strided_store(state, base_addr, stride, vs3, eew, id)
        }
        VectorOp::VStoreIndexOrd | VectorOp::VStoreIndexUnord => {
            exec_indexed_store(state, base_addr, vs3, eew, id)
        }
        VectorOp::VStoreMask => exec_mask_store(state, base_addr, vs3, id),
        VectorOp::VStoreWholeReg => exec_whole_reg_store(state, base_addr, vs3, eew, id),
        _ => Ok(0),
    }
}

/// Returns true if the given `VectorOp` is a vector load.
pub const fn is_vec_load(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VLoadUnit
            | VectorOp::VLoadFF
            | VectorOp::VLoadStride
            | VectorOp::VLoadIndexOrd
            | VectorOp::VLoadIndexUnord
            | VectorOp::VLoadMask
            | VectorOp::VLoadWholeReg
    )
}

/// Returns true if the given `VectorOp` is a vector store.
pub const fn is_vec_store(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VStoreUnit
            | VectorOp::VStoreStride
            | VectorOp::VStoreIndexOrd
            | VectorOp::VStoreIndexUnord
            | VectorOp::VStoreMask
            | VectorOp::VStoreWholeReg
    )
}

/// Returns true if the given `VectorOp` is any vector memory operation.
pub const fn is_vec_mem(op: VectorOp) -> bool {
    is_vec_load(op) || is_vec_store(op)
}

/// Generate per-element address micro-ops for a vector memory instruction.
///
/// This computes the virtual address for each active element without performing
/// any memory access. The O3 backend routes each micro-op through the real
/// Memory1 → Memory2 → Writeback pipeline for accurate cache/TLB timing.
///
/// For stores, `store_data` carries the element value from the arch VPR
/// (already written by functional execution).
///
/// For loads, `store_data` is 0 — the functional execution already wrote the
/// correct values to the arch VPR; the micro-ops exist only for timing.
#[must_use]
pub fn generate_element_addrs(
    state: &CoreCtx<'_>,
    ex_result: &ExMem1Entry,
    vec_op: VectorOp,
) -> Vec<VecMemAddrOp> {
    let is_store = is_vec_store(vec_op);
    let eew = parse_eew_from_ctrl(&ex_result.ctrl);
    let base_addr = ex_result.alu; // rs1 base address (already computed)

    match vec_op {
        VectorOp::VLoadUnit | VectorOp::VStoreUnit | VectorOp::VLoadFF => {
            gen_unit_stride_addrs(state, base_addr, eew, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadStride | VectorOp::VStoreStride => {
            let stride = ex_result.store_data as i64; // rs2 holds stride for strided ops
            // For stores, ex_result.store_data was overwritten with rs2 (stride).
            // The actual store data comes from the VPR (already executed functionally).
            gen_strided_addrs(state, base_addr, stride, eew, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadIndexOrd
        | VectorOp::VLoadIndexUnord
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
            gen_indexed_addrs(state, base_addr, eew, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadMask | VectorOp::VStoreMask => {
            gen_mask_addrs(state, base_addr, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadWholeReg | VectorOp::VStoreWholeReg => {
            gen_whole_reg_addrs(state, base_addr, &ex_result.ctrl, is_store)
        }
        _ => Vec::new(),
    }
}

/// Extract the effective element width from control signals.
const fn parse_eew_from_ctrl(ctrl: &crate::core::pipeline::signals::ControlSignals) -> Sew {
    ctrl.vec_eew
}

/// Generate unit-stride element addresses.
fn gen_unit_stride_addrs(
    state: &CoreCtx<'_>,
    base: u64,
    eew: Sew,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Vec::new();
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let vm = ctrl.vm;
    let vd = ctrl.vd;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
    let mut ops = Vec::with_capacity(vl * nf.fields_usize());

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        for seg in 0..nf.fields_usize() {
            let addr =
                base.wrapping_add(((i * nf.fields_usize() + seg) as u64).wrapping_mul(eew_bytes));
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            let store_data =
                if is_store { state.hart.regs.vpr().read_element(dest, ElemIdx::new(i), eew) } else { 0 };
            ops.push(VecMemAddrOp {
                vaddr: VirtAddr::new(addr),
                store_data,
                elem_idx: ElemIdx::new(i),
                eew,
                vd_phys: VecPhysReg::ZERO, // Filled in by O3Engine from rename map
            });
        }
    }
    ops
}

/// Generate strided element addresses.
fn gen_strided_addrs(
    state: &CoreCtx<'_>,
    base: u64,
    stride: i64,
    eew: Sew,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Vec::new();
    };
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let vm = ctrl.vm;
    let vd = ctrl.vd;
    let eew_bytes = eew.bytes() as u64;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
    let mut ops = Vec::with_capacity(vl * nf.fields_usize());

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf.fields_usize() {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            let store_data =
                if is_store { state.hart.regs.vpr().read_element(dest, ElemIdx::new(i), eew) } else { 0 };
            ops.push(VecMemAddrOp {
                vaddr: VirtAddr::new(addr),
                store_data,
                elem_idx: ElemIdx::new(i),
                eew,
                vd_phys: VecPhysReg::ZERO,
            });
        }
    }
    ops
}

/// Generate indexed element addresses.
fn gen_indexed_addrs(
    state: &CoreCtx<'_>,
    base: u64,
    eew: Sew,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let vtype = parse_vtype(state.hart.csrs.vtype);
    if vtype.vill {
        return Vec::new();
    }
    let vl = state.hart.csrs.vl as usize;
    let vstart = state.hart.csrs.vstart as usize;
    let vm = ctrl.vm;
    let vs2 = ctrl.vs2;
    let vd = ctrl.vd;
    let data_sew = vtype.vsew;
    let idx_eew = eew;
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let data_bytes = data_sew.bytes() as u64;
    let data_emul = Emul::compute(data_sew, data_sew, vtype.vlmul);
    let mut ops = Vec::with_capacity(vl * nf.fields_usize());

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        let offset = state.hart.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf.fields_usize() {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * data_emul.regs());
            let store_data = if is_store {
                state.hart.regs.vpr().read_element(dest, ElemIdx::new(i), data_sew)
            } else {
                0
            };
            ops.push(VecMemAddrOp {
                vaddr: VirtAddr::new(addr),
                store_data,
                elem_idx: ElemIdx::new(i),
                eew: data_sew,
                vd_phys: VecPhysReg::ZERO,
            });
        }
    }
    ops
}

/// Generate mask load/store element addresses.
fn gen_mask_addrs(
    state: &CoreCtx<'_>,
    base: u64,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let vl = state.hart.csrs.vl as usize;
    let num_bytes = vl.div_ceil(8);
    let vd = ctrl.vd;
    let mut ops = Vec::with_capacity(num_bytes);

    for i in 0..num_bytes {
        let addr = base.wrapping_add(i as u64);
        let store_data =
            if is_store { state.hart.regs.vpr().read_element(vd, ElemIdx::new(i), Sew::E8) } else { 0 };
        ops.push(VecMemAddrOp {
            vaddr: VirtAddr::new(addr),
            store_data,
            elem_idx: ElemIdx::new(i),
            eew: Sew::E8,
            vd_phys: VecPhysReg::ZERO,
        });
    }
    ops
}

/// Generate whole-register load/store element addresses.
fn gen_whole_reg_addrs(
    state: &CoreCtx<'_>,
    base: u64,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let nreg = (ctrl.vec_nf as usize) + 1;
    let vlen_bytes = state.hart.regs.vpr().vlen().bytes();
    let total_bytes = nreg * vlen_bytes;
    let vd = ctrl.vd;
    let mut ops = Vec::with_capacity(total_bytes);

    for i in 0..total_bytes {
        let addr = base.wrapping_add(i as u64);
        let reg_offset = i / vlen_bytes;
        let byte_offset = i % vlen_bytes;
        let src = VRegIdx::new(vd.as_u8() + reg_offset as u8);
        let store_data = if is_store {
            state.hart.regs.vpr().read_element(src, ElemIdx::new(byte_offset), Sew::E8)
        } else {
            0
        };
        ops.push(VecMemAddrOp {
            vaddr: VirtAddr::new(addr),
            store_data,
            elem_idx: ElemIdx::new(byte_offset),
            eew: Sew::E8,
            vd_phys: VecPhysReg::ZERO,
        });
    }
    ops
}

/// Compute the physical destination register for a given element.
///
/// For non-segment ops: `vd_phys[elem / elements_per_reg]`
/// For segment ops: `vd_phys[seg * emul_regs + elem / elements_per_reg]`
const fn compute_vd_phys(
    vd_phys: &[VecPhysReg; 8],
    vd_count: u8,
    elem: usize,
    seg: usize,
    emul_regs: usize,
    elements_per_reg: usize,
) -> VecPhysReg {
    let reg_in_seg = if elements_per_reg > 0 { elem / elements_per_reg } else { 0 };
    let idx = seg * emul_regs + reg_in_seg;
    if idx < vd_count as usize { vd_phys[idx] } else { VecPhysReg::ZERO }
}

/// Generate per-element address micro-ops using any `VectorRegFile` implementation.
///
/// This is the O3 backend's address generation path. Unlike [`generate_element_addrs`],
/// it reads store data and index values from the provided `VectorRegFile` (typically a
/// `VecPrfView` backed by the physical register file) and computes the correct
/// physical destination register for each element.
///
/// Does NOT perform any memory access.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn generate_element_addrs_vrf<V: VectorRegFile>(
    vrf: &V,
    base_addr: u64,
    stride: i64,
    ctrl: &ControlSignals,
    vtype_bits: u64,
    vl: usize,
    vstart: usize,
    vec_op: VectorOp,
    vd_phys: &[VecPhysReg; 8],
    vd_count: u8,
) -> Vec<VecMemAddrOp> {
    let vtype = parse_vtype(vtype_bits);
    if vtype.vill {
        return Vec::new();
    }

    let is_store = is_vec_store(vec_op);
    let eew = ctrl.vec_eew;
    let vm = ctrl.vm;
    let vd = ctrl.vd;
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let vlen = vrf.vlen();

    match vec_op {
        VectorOp::VLoadUnit | VectorOp::VStoreUnit | VectorOp::VLoadFF => {
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
                    let phys =
                        compute_vd_phys(vd_phys, vd_count, i, seg, emul_regs, elements_per_reg);
                    ops.push(VecMemAddrOp {
                        vaddr: VirtAddr::new(addr),
                        store_data,
                        elem_idx: ElemIdx::new(i),
                        eew,
                        vd_phys: phys,
                    });
                }
            }
            ops
        }
        VectorOp::VLoadStride | VectorOp::VStoreStride => {
            let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
            let eew_bytes = eew.bytes() as u64;
            let elements_per_reg = vlen.bits() / (eew.bytes() * 8);
            let emul_regs = emul.regs() as usize;
            let mut ops = Vec::with_capacity(vl * nf.fields_usize());
            for i in vstart..vl {
                if !is_element_active_vrf(vrf, i, vm) {
                    continue;
                }
                let elem_base = base_addr.wrapping_add((i as i64).wrapping_mul(stride) as u64);
                for seg in 0..nf.fields_usize() {
                    let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
                    let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
                    let store_data =
                        if is_store { vrf.read_element(dest, ElemIdx::new(i), eew) } else { 0 };
                    let phys =
                        compute_vd_phys(vd_phys, vd_count, i, seg, emul_regs, elements_per_reg);
                    ops.push(VecMemAddrOp {
                        vaddr: VirtAddr::new(addr),
                        store_data,
                        elem_idx: ElemIdx::new(i),
                        eew,
                        vd_phys: phys,
                    });
                }
            }
            ops
        }
        VectorOp::VLoadIndexOrd
        | VectorOp::VLoadIndexUnord
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
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
                    let phys =
                        compute_vd_phys(vd_phys, vd_count, i, seg, emul_regs, elements_per_reg);
                    ops.push(VecMemAddrOp {
                        vaddr: VirtAddr::new(addr),
                        store_data,
                        elem_idx: ElemIdx::new(i),
                        eew: data_sew,
                        vd_phys: phys,
                    });
                }
            }
            ops
        }
        VectorOp::VLoadMask | VectorOp::VStoreMask => {
            let num_bytes = vl.div_ceil(8);
            let elements_per_reg = vlen.bits() / 8; // E8
            let mut ops = Vec::with_capacity(num_bytes);
            for i in 0..num_bytes {
                let addr = base_addr.wrapping_add(i as u64);
                let store_data =
                    if is_store { vrf.read_element(vd, ElemIdx::new(i), Sew::E8) } else { 0 };
                let phys = compute_vd_phys(vd_phys, vd_count, i, 0, 1, elements_per_reg);
                ops.push(VecMemAddrOp {
                    vaddr: VirtAddr::new(addr),
                    store_data,
                    elem_idx: ElemIdx::new(i),
                    eew: Sew::E8,
                    vd_phys: phys,
                });
            }
            ops
        }
        VectorOp::VLoadWholeReg | VectorOp::VStoreWholeReg => {
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
                let phys = if reg_offset < vd_count as usize {
                    vd_phys[reg_offset]
                } else {
                    VecPhysReg::ZERO
                };
                ops.push(VecMemAddrOp {
                    vaddr: VirtAddr::new(addr),
                    store_data,
                    elem_idx: ElemIdx::new(byte_offset),
                    eew: Sew::E8,
                    vd_phys: phys,
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

/// Get `(vl, vstart)` for the current vector configuration.
///
/// Returns `None` if vtype is illegal (vill=1).
const fn get_vec_cfg(state: &CoreCtx<'_>) -> Option<(usize, usize)> {
    let vtype = parse_vtype(state.hart.csrs.vtype);
    if vtype.vill {
        return None;
    }
    Some((state.hart.csrs.vl as usize, state.hart.csrs.vstart as usize))
}

/// Check if element `i` is active under the current mask.
fn is_element_active(state: &CoreCtx<'_>, i: usize, vm: bool) -> bool {
    if vm {
        // vm=1 means unmasked — all elements active
        return true;
    }
    state.hart.regs.vpr().read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Translates a vector-element address synchronously.
///
/// The in-order vector unit executes element-by-element atomically during
/// the execute stage, so it cannot park on a TLB miss the way the pipelined
/// LSU does. If translation needs a page-table walk this function surfaces
/// it as the appropriate page fault — the trap commits, the OS handler
/// installs the PTE, and the re-issue path warms the TLB before retrying.
fn translate_vector_element(
    state: &mut CoreCtx<'_>,
    vaddr: u64,
    access: AccessType,
    size: u64,
) -> Result<crate::common::PhysAddr, Trap> {
    use crate::sim::state::memory::TranslateResult;
    match state.translate(VirtAddr::new(vaddr), access, size) {
        TranslateResult::Ready(r) => {
            if let Some(trap) = r.trap {
                Err(trap)
            } else {
                Ok(r.paddr)
            }
        }
        TranslateResult::NeedPte { .. } => Err(match access {
            AccessType::Read => Trap::LoadPageFault(vaddr),
            AccessType::Write => Trap::StorePageFault(vaddr),
            AccessType::Fetch => Trap::InstructionPageFault(vaddr),
        }),
    }
}

/// Read a single element from memory at `vaddr` with the given EEW.
///
/// Reads bytes via the RAM fast-path pointer (vector memory ops are
/// architecturally not defined for MMIO regions; a non-RAM address
/// surfaces zero, which the encoded operation either consumes or
/// faults on at the protection check above).
fn mem_read_element(state: &mut CoreCtx<'_>, vaddr: u64, eew: Sew) -> Result<u64, Trap> {
    let size = eew.bytes() as u64;
    let paddr = translate_vector_element(state, vaddr, AccessType::Read, size)?;
    let raw = paddr.val();
    let region = state.bus.ram_region().filter(|r| r.contains(raw, size));
    // Architecturally invalid vector ops against MMIO return zero (None branch).
    let val = region.map_or(0, |r| {
        // SAFETY: `RamRegion::contains(raw, size)` bounds-checks the access.
        unsafe {
            match eew {
                Sew::E8 => u64::from(*r.ptr(raw)),
                Sew::E16 => u64::from(r.ptr(raw).cast::<u16>().read_unaligned()),
                Sew::E32 => u64::from(r.ptr(raw).cast::<u32>().read_unaligned()),
                Sew::E64 => r.ptr(raw).cast::<u64>().read_unaligned(),
            }
        }
    });
    Ok(val)
}

/// Write a single element to memory at `vaddr` with the given EEW.
///
/// Published as a write by this hart. Non-RAM addresses are silently
/// dropped — vector stores to MMIO are not architecturally defined.
fn mem_write_element(state: &mut CoreCtx<'_>, vaddr: u64, eew: Sew, val: u64) -> Result<(), Trap> {
    let size = eew.bytes() as u64;
    let paddr = translate_vector_element(state, vaddr, AccessType::Write, size)?;
    let width = match eew {
        Sew::E8 => MemWidth::Byte,
        Sew::E16 => MemWidth::Half,
        Sew::E32 => MemWidth::Word,
        Sew::E64 => MemWidth::Double,
    };
    state.publish_write(paddr, val, width);
    Ok(())
}

/// Execute a unit-stride vector load: `addr[i] = base + i * eew_bytes`.
fn exec_unit_stride_load(
    state: &mut CoreCtx<'_>,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Ok(0);
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = (id.ctrl.vec_nf as usize) + 1; // nf encoding is nf-1
    let vm = id.ctrl.vm;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            let val = mem_read_element(state, addr, eew).inspect_err(|_t| {
                state.hart.csrs.vstart = i as u64;
            })?;
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            state.hart.regs.vpr_mut().write_element(dest, ElemIdx::new(i), eew, val);
        }
    }

    Ok(0)
}

/// Execute a fault-only-first vector load.
///
/// Element 0 traps normally. For elements > 0, a trap sets `vl = i` and stops
/// without raising the exception.
fn exec_fault_first_load(
    state: &mut CoreCtx<'_>,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Ok(0);
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    'elements: for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            match mem_read_element(state, addr, eew) {
                Ok(val) => {
                    let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
                    state.hart.regs.vpr_mut().write_element(dest, ElemIdx::new(i), eew, val);
                }
                Err(trap) => {
                    if i == 0 && seg == 0 {
                        state.hart.csrs.vstart = 0;
                        return Err(trap);
                    }
                    // Trim vl to the faulting element index; drop its segment.
                    state.hart.csrs.vl = i as u64;
                    break 'elements;
                }
            }
        }
    }

    Ok(0)
}

/// Execute a strided vector load: `addr[i] = base + i * stride`.
fn exec_strided_load(
    state: &mut CoreCtx<'_>,
    base: u64,
    stride: i64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Ok(0);
    };
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let eew_bytes = eew.bytes() as u64;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let val = mem_read_element(state, addr, eew).inspect_err(|_t| {
                state.hart.csrs.vstart = i as u64;
            })?;
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            state.hart.regs.vpr_mut().write_element(dest, ElemIdx::new(i), eew, val);
        }
    }

    Ok(0)
}

/// Execute an indexed vector load: `addr[i] = base + vs2[i]`.
///
/// The index vector `vs2` has element width = EEW (from the instruction encoding).
/// The data loaded has element width = SEW (from current vtype).
fn exec_indexed_load(
    state: &mut CoreCtx<'_>,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let vtype = parse_vtype(state.hart.csrs.vtype);
    if vtype.vill {
        return Ok(0);
    }
    let vl = state.hart.csrs.vl as usize;
    let vstart = state.hart.csrs.vstart as usize;
    let vm = id.ctrl.vm;
    let vs2 = id.ctrl.vs2;
    let data_sew = vtype.vsew; // data element width = SEW
    let idx_eew = eew; // index element width = EEW from instruction
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let data_bytes = data_sew.bytes() as u64;
    // Data EMUL: field spacing for the destination register group.
    let data_emul = Emul::compute(data_sew, data_sew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        let offset = state.hart.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let val = mem_read_element(state, addr, data_sew).inspect_err(|_t| {
                state.hart.csrs.vstart = i as u64;
            })?;
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * data_emul.regs());
            state.hart.regs.vpr_mut().write_element(dest, ElemIdx::new(i), data_sew, val);
        }
    }

    Ok(0)
}

/// Execute a mask load (`vlm.v`): loads `ceil(vl/8)` bytes into `vd`.
///
/// Mask loads always use EEW=8 and ignore vtype SEW. The mask is stored
/// as a bitfield in the destination register.
fn exec_mask_load(
    state: &mut CoreCtx<'_>,
    base: u64,
    vd: VRegIdx,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let _ = id; // mask load ignores most fields
    let vl = state.hart.csrs.vl as usize;
    let num_bytes = vl.div_ceil(8);

    for i in 0..num_bytes {
        let addr = base.wrapping_add(i as u64);
        let val = mem_read_element(state, addr, Sew::E8).inspect_err(|_t| {
            state.hart.csrs.vstart = i as u64;
        })?;
        state.hart.regs.vpr_mut().write_element(vd, ElemIdx::new(i), Sew::E8, val);
    }

    Ok(0)
}

/// Execute a whole-register load (`vl1re8`, `vl2re8`, etc.).
///
/// Loads `nf` complete registers (ignores vl, vtype, mask). `nf` is encoded
/// in bits 31:29 as `nf - 1`. Loads `nf * VLEN/8` bytes sequentially.
fn exec_whole_reg_load(
    state: &mut CoreCtx<'_>,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let nreg = (id.ctrl.vec_nf as usize) + 1; // number of registers to load
    let vlen_bytes = state.hart.regs.vpr().vlen().bytes();
    let total_bytes = nreg * vlen_bytes;
    let _ = eew; // EEW is used for hint purposes; data is byte-level

    for i in 0..total_bytes {
        let addr = base.wrapping_add(i as u64);
        let val = mem_read_element(state, addr, Sew::E8).inspect_err(|_t| {
            state.hart.csrs.vstart = i as u64;
        })?;
        let reg_offset = i / vlen_bytes;
        let byte_offset = i % vlen_bytes;
        let dest = VRegIdx::new(vd.as_u8() + reg_offset as u8);
        state.hart.regs.vpr_mut().write_element(dest, ElemIdx::new(byte_offset), Sew::E8, val);
    }

    Ok(0)
}

/// Execute a unit-stride vector store: `addr[i] = base + i * eew_bytes`.
fn exec_unit_stride_store(
    state: &mut CoreCtx<'_>,
    base: u64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Ok(0);
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            let src = VRegIdx::new(vs3.as_u8() + (seg as u8) * emul.regs());
            let val = state.hart.regs.vpr().read_element(src, ElemIdx::new(i), eew);
            mem_write_element(state, addr, eew, val).inspect_err(|_t| {
                state.hart.csrs.vstart = i as u64;
            })?;
        }
    }

    Ok(0)
}

/// Execute a strided vector store: `addr[i] = base + i * stride`.
fn exec_strided_store(
    state: &mut CoreCtx<'_>,
    base: u64,
    stride: i64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(state) else {
        return Ok(0);
    };
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let eew_bytes = eew.bytes() as u64;
    let vtype = parse_vtype(state.hart.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let src = VRegIdx::new(vs3.as_u8() + (seg as u8) * emul.regs());
            let val = state.hart.regs.vpr().read_element(src, ElemIdx::new(i), eew);
            mem_write_element(state, addr, eew, val).inspect_err(|_t| {
                state.hart.csrs.vstart = i as u64;
            })?;
        }
    }

    Ok(0)
}

/// Execute an indexed vector store: `addr[i] = base + vs2[i]`.
fn exec_indexed_store(
    state: &mut CoreCtx<'_>,
    base: u64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let vtype = parse_vtype(state.hart.csrs.vtype);
    if vtype.vill {
        return Ok(0);
    }
    let vl = state.hart.csrs.vl as usize;
    let vstart = state.hart.csrs.vstart as usize;
    let vm = id.ctrl.vm;
    let vs2 = id.ctrl.vs2;
    let data_sew = vtype.vsew;
    let idx_eew = eew;
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let data_bytes = data_sew.bytes() as u64;
    let data_emul = Emul::compute(data_sew, data_sew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(state, i, vm) {
            continue;
        }
        let offset = state.hart.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let src = VRegIdx::new(vs3.as_u8() + (seg as u8) * data_emul.regs());
            let val = state.hart.regs.vpr().read_element(src, ElemIdx::new(i), data_sew);
            mem_write_element(state, addr, data_sew, val).inspect_err(|_t| {
                state.hart.csrs.vstart = i as u64;
            })?;
        }
    }

    Ok(0)
}

/// Execute a mask store (`vsm.v`): stores `ceil(vl/8)` bytes from `vs3`.
fn exec_mask_store(
    state: &mut CoreCtx<'_>,
    base: u64,
    vs3: VRegIdx,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let _ = id;
    let vl = state.hart.csrs.vl as usize;
    let num_bytes = vl.div_ceil(8);

    for i in 0..num_bytes {
        let addr = base.wrapping_add(i as u64);
        let val = state.hart.regs.vpr().read_element(vs3, ElemIdx::new(i), Sew::E8);
        mem_write_element(state, addr, Sew::E8, val).inspect_err(|_t| {
            state.hart.csrs.vstart = i as u64;
        })?;
    }

    Ok(0)
}

/// Execute a whole-register store (`vs1r`, `vs2r`, etc.).
///
/// Stores `nf` complete registers (ignores vl, vtype, mask).
fn exec_whole_reg_store(
    state: &mut CoreCtx<'_>,
    base: u64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let nreg = (id.ctrl.vec_nf as usize) + 1;
    let vlen_bytes = state.hart.regs.vpr().vlen().bytes();
    let total_bytes = nreg * vlen_bytes;
    let _ = eew;

    for i in 0..total_bytes {
        let addr = base.wrapping_add(i as u64);
        let reg_offset = i / vlen_bytes;
        let byte_offset = i % vlen_bytes;
        let src = VRegIdx::new(vs3.as_u8() + reg_offset as u8);
        let val = state.hart.regs.vpr().read_element(src, ElemIdx::new(byte_offset), Sew::E8);
        mem_write_element(state, addr, Sew::E8, val).inspect_err(|_t| {
            state.hart.csrs.vstart = i as u64;
        })?;
    }

    Ok(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_is_vec_load() {
        assert!(is_vec_load(VectorOp::VLoadUnit));
        assert!(is_vec_load(VectorOp::VLoadFF));
        assert!(is_vec_load(VectorOp::VLoadStride));
        assert!(is_vec_load(VectorOp::VLoadIndexOrd));
        assert!(is_vec_load(VectorOp::VLoadIndexUnord));
        assert!(is_vec_load(VectorOp::VLoadMask));
        assert!(is_vec_load(VectorOp::VLoadWholeReg));
        assert!(!is_vec_load(VectorOp::VStoreUnit));
        assert!(!is_vec_load(VectorOp::VAdd));
        assert!(!is_vec_load(VectorOp::None));
    }

    #[test]
    fn test_is_vec_store() {
        assert!(is_vec_store(VectorOp::VStoreUnit));
        assert!(is_vec_store(VectorOp::VStoreStride));
        assert!(is_vec_store(VectorOp::VStoreIndexOrd));
        assert!(is_vec_store(VectorOp::VStoreIndexUnord));
        assert!(is_vec_store(VectorOp::VStoreMask));
        assert!(is_vec_store(VectorOp::VStoreWholeReg));
        assert!(!is_vec_store(VectorOp::VLoadUnit));
        assert!(!is_vec_store(VectorOp::VAdd));
    }

    #[test]
    fn test_is_vec_mem() {
        assert!(is_vec_mem(VectorOp::VLoadUnit));
        assert!(is_vec_mem(VectorOp::VStoreUnit));
        assert!(!is_vec_mem(VectorOp::VAdd));
        assert!(!is_vec_mem(VectorOp::None));
    }
}

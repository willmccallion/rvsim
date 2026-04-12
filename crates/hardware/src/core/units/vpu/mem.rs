//! Vector memory operations.
//!
//! Implements per-element address computation and memory access for all vector
//! load/store variants: unit-stride, strided, indexed, mask, whole-register,
//! and fault-only-first. All accesses go through the CPU's address translation
//! and bus interface.

use crate::common::{AccessType, Trap, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::signals::VectorOp;
use crate::core::units::vpu::types::{ElemIdx, Emul, Nf, Sew, VRegIdx, VecPhysReg, parse_vtype};

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
pub fn execute_vec_load(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let eew = id.ctrl.vec_eew;
    let vd = id.ctrl.vd;
    let base_addr = id.rv1; // rs1 holds the base address

    match id.ctrl.vec_op {
        VectorOp::VLoadUnit => exec_unit_stride_load(cpu, base_addr, vd, eew, id),
        VectorOp::VLoadFF => exec_fault_first_load(cpu, base_addr, vd, eew, id),
        VectorOp::VLoadStride => {
            let stride = id.rv2 as i64;
            exec_strided_load(cpu, base_addr, stride, vd, eew, id)
        }
        VectorOp::VLoadIndexOrd | VectorOp::VLoadIndexUnord => {
            exec_indexed_load(cpu, base_addr, vd, eew, id)
        }
        VectorOp::VLoadMask => exec_mask_load(cpu, base_addr, vd, id),
        VectorOp::VLoadWholeReg => exec_whole_reg_load(cpu, base_addr, vd, eew, id),
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
pub fn execute_vec_store(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let eew = id.ctrl.vec_eew;
    let vs3 = id.ctrl.vd; // vd field encodes vs3 (store data source) for stores
    let base_addr = id.rv1;

    match id.ctrl.vec_op {
        VectorOp::VStoreUnit => exec_unit_stride_store(cpu, base_addr, vs3, eew, id),
        VectorOp::VStoreStride => {
            let stride = id.rv2 as i64;
            exec_strided_store(cpu, base_addr, stride, vs3, eew, id)
        }
        VectorOp::VStoreIndexOrd | VectorOp::VStoreIndexUnord => {
            exec_indexed_store(cpu, base_addr, vs3, eew, id)
        }
        VectorOp::VStoreMask => exec_mask_store(cpu, base_addr, vs3, id),
        VectorOp::VStoreWholeReg => exec_whole_reg_store(cpu, base_addr, vs3, eew, id),
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

// ── Address generation for O3 pipeline ───────────────────────────────────────

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
    cpu: &Cpu,
    ex_result: &ExMem1Entry,
    vec_op: VectorOp,
) -> Vec<VecMemAddrOp> {
    let is_store = is_vec_store(vec_op);
    let eew = parse_eew_from_ctrl(&ex_result.ctrl);
    let base_addr = ex_result.alu; // rs1 base address (already computed)

    match vec_op {
        VectorOp::VLoadUnit | VectorOp::VStoreUnit | VectorOp::VLoadFF => {
            gen_unit_stride_addrs(cpu, base_addr, eew, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadStride | VectorOp::VStoreStride => {
            let stride = ex_result.store_data as i64; // rs2 holds stride for strided ops
            // For stores, ex_result.store_data was overwritten with rs2 (stride).
            // The actual store data comes from the VPR (already executed functionally).
            gen_strided_addrs(cpu, base_addr, stride, eew, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadIndexOrd | VectorOp::VLoadIndexUnord
        | VectorOp::VStoreIndexOrd | VectorOp::VStoreIndexUnord => {
            gen_indexed_addrs(cpu, base_addr, eew, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadMask | VectorOp::VStoreMask => {
            gen_mask_addrs(cpu, base_addr, &ex_result.ctrl, is_store)
        }
        VectorOp::VLoadWholeReg | VectorOp::VStoreWholeReg => {
            gen_whole_reg_addrs(cpu, base_addr, &ex_result.ctrl, is_store)
        }
        _ => Vec::new(),
    }
}

/// Extract the effective element width from control signals.
fn parse_eew_from_ctrl(ctrl: &crate::core::pipeline::signals::ControlSignals) -> Sew {
    ctrl.vec_eew
}

/// Generate unit-stride element addresses.
fn gen_unit_stride_addrs(
    cpu: &Cpu,
    base: u64,
    eew: Sew,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Vec::new();
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let vm = ctrl.vm;
    let vd = ctrl.vd;
    let vtype = parse_vtype(cpu.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
    let mut ops = Vec::with_capacity(vl * nf.fields_usize());

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        for seg in 0..nf.fields_usize() {
            let addr = base.wrapping_add(((i * nf.fields_usize() + seg) as u64).wrapping_mul(eew_bytes));
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            let store_data = if is_store {
                cpu.regs.vpr().read_element(dest, ElemIdx::new(i), eew)
            } else {
                0
            };
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
    cpu: &Cpu,
    base: u64,
    stride: i64,
    eew: Sew,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Vec::new();
    };
    let nf = Nf::from_encoding(ctrl.vec_nf);
    let vm = ctrl.vm;
    let vd = ctrl.vd;
    let eew_bytes = eew.bytes() as u64;
    let vtype = parse_vtype(cpu.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
    let mut ops = Vec::with_capacity(vl * nf.fields_usize());

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf.fields_usize() {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            let store_data = if is_store {
                cpu.regs.vpr().read_element(dest, ElemIdx::new(i), eew)
            } else {
                0
            };
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
    cpu: &Cpu,
    base: u64,
    eew: Sew,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return Vec::new();
    }
    let vl = cpu.csrs.vl as usize;
    let vstart = cpu.csrs.vstart as usize;
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
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let offset = cpu.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf.fields_usize() {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * data_emul.regs());
            let store_data = if is_store {
                cpu.regs.vpr().read_element(dest, ElemIdx::new(i), data_sew)
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
    cpu: &Cpu,
    base: u64,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let vl = cpu.csrs.vl as usize;
    let num_bytes = vl.div_ceil(8);
    let vd = ctrl.vd;
    let mut ops = Vec::with_capacity(num_bytes);

    for i in 0..num_bytes {
        let addr = base.wrapping_add(i as u64);
        let store_data = if is_store {
            cpu.regs.vpr().read_element(vd, ElemIdx::new(i), Sew::E8)
        } else {
            0
        };
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
    cpu: &Cpu,
    base: u64,
    ctrl: &crate::core::pipeline::signals::ControlSignals,
    is_store: bool,
) -> Vec<VecMemAddrOp> {
    let nreg = (ctrl.vec_nf as usize) + 1;
    let vlen_bytes = cpu.regs.vpr().vlen().bytes();
    let total_bytes = nreg * vlen_bytes;
    let vd = ctrl.vd;
    let mut ops = Vec::with_capacity(total_bytes);

    for i in 0..total_bytes {
        let addr = base.wrapping_add(i as u64);
        let reg_offset = i / vlen_bytes;
        let byte_offset = i % vlen_bytes;
        let src = VRegIdx::new(vd.as_u8() + reg_offset as u8);
        let store_data = if is_store {
            cpu.regs.vpr().read_element(src, ElemIdx::new(byte_offset), Sew::E8)
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

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Get `(vl, vstart)` for the current vector configuration.
///
/// Returns `None` if vtype is illegal (vill=1).
const fn get_vec_cfg(cpu: &Cpu) -> Option<(usize, usize)> {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return None;
    }
    Some((cpu.csrs.vl as usize, cpu.csrs.vstart as usize))
}

/// Check if element `i` is active under the current mask.
fn is_element_active(cpu: &Cpu, i: usize, vm: bool) -> bool {
    if vm {
        // vm=1 means unmasked — all elements active
        return true;
    }
    cpu.regs.vpr().read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Read a single element from memory at `vaddr` with the given EEW.
fn mem_read_element(cpu: &mut Cpu, vaddr: u64, eew: Sew) -> Result<u64, Trap> {
    let size = eew.bytes() as u64;
    let tr = cpu.translate(VirtAddr::new(vaddr), AccessType::Read, size);
    if let Some(trap) = tr.trap {
        return Err(trap);
    }
    let paddr = tr.paddr;
    let val = match eew {
        Sew::E8 => u64::from(cpu.bus.bus.read_u8(paddr)),
        Sew::E16 => u64::from(cpu.bus.bus.read_u16(paddr)),
        Sew::E32 => u64::from(cpu.bus.bus.read_u32(paddr)),
        Sew::E64 => cpu.bus.bus.read_u64(paddr),
    };
    Ok(val)
}

/// Write a single element to memory at `vaddr` with the given EEW.
fn mem_write_element(cpu: &mut Cpu, vaddr: u64, eew: Sew, val: u64) -> Result<(), Trap> {
    let size = eew.bytes() as u64;
    let tr = cpu.translate(VirtAddr::new(vaddr), AccessType::Write, size);
    if let Some(trap) = tr.trap {
        return Err(trap);
    }
    let paddr = tr.paddr;
    match eew {
        Sew::E8 => cpu.bus.bus.write_u8(paddr, val as u8),
        Sew::E16 => cpu.bus.bus.write_u16(paddr, val as u16),
        Sew::E32 => cpu.bus.bus.write_u32(paddr, val as u32),
        Sew::E64 => cpu.bus.bus.write_u64(paddr, val),
    }
    Ok(())
}

// ── Unit-stride load ─────────────────────────────────────────────────────────

/// Execute a unit-stride vector load: `addr[i] = base + i * eew_bytes`.
fn exec_unit_stride_load(
    cpu: &mut Cpu,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Ok(0);
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = (id.ctrl.vec_nf as usize) + 1; // nf encoding is nf-1
    let vm = id.ctrl.vm;
    let vtype = parse_vtype(cpu.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);
    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            let val = mem_read_element(cpu, addr, eew)?;
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            cpu.regs.vpr_mut().write_element(dest, ElemIdx::new(i), eew, val);
        }
    }

    Ok(0)
}

// ── Fault-only-first load ────────────────────────────────────────────────────

/// Execute a fault-only-first vector load.
///
/// Element 0 traps normally. For elements > 0, a trap sets `vl = i` and stops
/// without raising the exception.
fn exec_fault_first_load(
    cpu: &mut Cpu,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Ok(0);
    };
    let eew_bytes = eew.bytes() as u64;
    let vm = id.ctrl.vm;

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let addr = base.wrapping_add((i as u64).wrapping_mul(eew_bytes));
        match mem_read_element(cpu, addr, eew) {
            Ok(val) => {
                cpu.regs.vpr_mut().write_element(vd, ElemIdx::new(i), eew, val);
            }
            Err(trap) => {
                if i == 0 {
                    // Element 0 faults propagate normally
                    return Err(trap);
                }
                // Elements > 0: silently trim vl
                cpu.csrs.vl = i as u64;
                break;
            }
        }
    }

    Ok(0)
}

// ── Strided load ─────────────────────────────────────────────────────────────

/// Execute a strided vector load: `addr[i] = base + i * stride`.
fn exec_strided_load(
    cpu: &mut Cpu,
    base: u64,
    stride: i64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Ok(0);
    };
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let eew_bytes = eew.bytes() as u64;
    let vtype = parse_vtype(cpu.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let val = mem_read_element(cpu, addr, eew)?;
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * emul.regs());
            cpu.regs.vpr_mut().write_element(dest, ElemIdx::new(i), eew, val);
        }
    }

    Ok(0)
}

// ── Indexed load ─────────────────────────────────────────────────────────────

/// Execute an indexed vector load: `addr[i] = base + vs2[i]`.
///
/// The index vector `vs2` has element width = EEW (from the instruction encoding).
/// The data loaded has element width = SEW (from current vtype).
fn exec_indexed_load(
    cpu: &mut Cpu,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return Ok(0);
    }
    let vl = cpu.csrs.vl as usize;
    let vstart = cpu.csrs.vstart as usize;
    let vm = id.ctrl.vm;
    let vs2 = id.ctrl.vs2;
    let data_sew = vtype.vsew; // data element width = SEW
    let idx_eew = eew; // index element width = EEW from instruction
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let data_bytes = data_sew.bytes() as u64;
    // Data EMUL: field spacing for the destination register group.
    let data_emul = Emul::compute(data_sew, data_sew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let offset = cpu.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let val = mem_read_element(cpu, addr, data_sew)?;
            let dest = VRegIdx::new(vd.as_u8() + (seg as u8) * data_emul.regs());
            cpu.regs.vpr_mut().write_element(dest, ElemIdx::new(i), data_sew, val);
        }
    }

    Ok(0)
}

// ── Mask load ────────────────────────────────────────────────────────────────

/// Execute a mask load (`vlm.v`): loads `ceil(vl/8)` bytes into `vd`.
///
/// Mask loads always use EEW=8 and ignore vtype SEW. The mask is stored
/// as a bitfield in the destination register.
fn exec_mask_load(
    cpu: &mut Cpu,
    base: u64,
    vd: VRegIdx,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let _ = id; // mask load ignores most fields
    let vl = cpu.csrs.vl as usize;
    let num_bytes = vl.div_ceil(8);

    for i in 0..num_bytes {
        let addr = base.wrapping_add(i as u64);
        let val = mem_read_element(cpu, addr, Sew::E8)?;
        cpu.regs.vpr_mut().write_element(vd, ElemIdx::new(i), Sew::E8, val);
    }

    Ok(0)
}

// ── Whole-register load ──────────────────────────────────────────────────────

/// Execute a whole-register load (`vl1re8`, `vl2re8`, etc.).
///
/// Loads `nf` complete registers (ignores vl, vtype, mask). `nf` is encoded
/// in bits 31:29 as `nf - 1`. Loads `nf * VLEN/8` bytes sequentially.
fn exec_whole_reg_load(
    cpu: &mut Cpu,
    base: u64,
    vd: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let nreg = (id.ctrl.vec_nf as usize) + 1; // number of registers to load
    let vlen_bytes = cpu.regs.vpr().vlen().bytes();
    let total_bytes = nreg * vlen_bytes;
    let _ = eew; // EEW is used for hint purposes; data is byte-level

    for i in 0..total_bytes {
        let addr = base.wrapping_add(i as u64);
        let val = mem_read_element(cpu, addr, Sew::E8)?;
        let reg_offset = i / vlen_bytes;
        let byte_offset = i % vlen_bytes;
        let dest = VRegIdx::new(vd.as_u8() + reg_offset as u8);
        cpu.regs.vpr_mut().write_element(dest, ElemIdx::new(byte_offset), Sew::E8, val);
    }

    Ok(0)
}

// ── Unit-stride store ────────────────────────────────────────────────────────

/// Execute a unit-stride vector store: `addr[i] = base + i * eew_bytes`.
fn exec_unit_stride_store(
    cpu: &mut Cpu,
    base: u64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Ok(0);
    };
    let eew_bytes = eew.bytes() as u64;
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let vtype = parse_vtype(cpu.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            let src = VRegIdx::new(vs3.as_u8() + (seg as u8) * emul.regs());
            let val = cpu.regs.vpr().read_element(src, ElemIdx::new(i), eew);
            mem_write_element(cpu, addr, eew, val)?;
        }
    }

    Ok(0)
}

// ── Strided store ────────────────────────────────────────────────────────────

/// Execute a strided vector store: `addr[i] = base + i * stride`.
fn exec_strided_store(
    cpu: &mut Cpu,
    base: u64,
    stride: i64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let Some((vl, vstart)) = get_vec_cfg(cpu) else {
        return Ok(0);
    };
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let vm = id.ctrl.vm;
    let eew_bytes = eew.bytes() as u64;
    let vtype = parse_vtype(cpu.csrs.vtype);
    let emul = Emul::compute(eew, vtype.vsew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let src = VRegIdx::new(vs3.as_u8() + (seg as u8) * emul.regs());
            let val = cpu.regs.vpr().read_element(src, ElemIdx::new(i), eew);
            mem_write_element(cpu, addr, eew, val)?;
        }
    }

    Ok(0)
}

// ── Indexed store ────────────────────────────────────────────────────────────

/// Execute an indexed vector store: `addr[i] = base + vs2[i]`.
fn exec_indexed_store(
    cpu: &mut Cpu,
    base: u64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return Ok(0);
    }
    let vl = cpu.csrs.vl as usize;
    let vstart = cpu.csrs.vstart as usize;
    let vm = id.ctrl.vm;
    let vs2 = id.ctrl.vs2;
    let data_sew = vtype.vsew;
    let idx_eew = eew;
    let nf = (id.ctrl.vec_nf as usize) + 1;
    let data_bytes = data_sew.bytes() as u64;
    let data_emul = Emul::compute(data_sew, data_sew, vtype.vlmul);

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let offset = cpu.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let src = VRegIdx::new(vs3.as_u8() + (seg as u8) * data_emul.regs());
            let val = cpu.regs.vpr().read_element(src, ElemIdx::new(i), data_sew);
            mem_write_element(cpu, addr, data_sew, val)?;
        }
    }

    Ok(0)
}

// ── Mask store ───────────────────────────────────────────────────────────────

/// Execute a mask store (`vsm.v`): stores `ceil(vl/8)` bytes from `vs3`.
fn exec_mask_store(
    cpu: &mut Cpu,
    base: u64,
    vs3: VRegIdx,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let _ = id;
    let vl = cpu.csrs.vl as usize;
    let num_bytes = vl.div_ceil(8);

    for i in 0..num_bytes {
        let addr = base.wrapping_add(i as u64);
        let val = cpu.regs.vpr().read_element(vs3, ElemIdx::new(i), Sew::E8);
        mem_write_element(cpu, addr, Sew::E8, val)?;
    }

    Ok(0)
}

// ── Whole-register store ─────────────────────────────────────────────────────

/// Execute a whole-register store (`vs1r`, `vs2r`, etc.).
///
/// Stores `nf` complete registers (ignores vl, vtype, mask).
fn exec_whole_reg_store(
    cpu: &mut Cpu,
    base: u64,
    vs3: VRegIdx,
    eew: Sew,
    id: &RenameIssueEntry,
) -> Result<u64, Trap> {
    let nreg = (id.ctrl.vec_nf as usize) + 1;
    let vlen_bytes = cpu.regs.vpr().vlen().bytes();
    let total_bytes = nreg * vlen_bytes;
    let _ = eew;

    for i in 0..total_bytes {
        let addr = base.wrapping_add(i as u64);
        let reg_offset = i / vlen_bytes;
        let byte_offset = i % vlen_bytes;
        let src = VRegIdx::new(vs3.as_u8() + reg_offset as u8);
        let val = cpu.regs.vpr().read_element(src, ElemIdx::new(byte_offset), Sew::E8);
        mem_write_element(cpu, addr, Sew::E8, val)?;
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

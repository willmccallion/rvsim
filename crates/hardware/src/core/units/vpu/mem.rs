//! Vector memory operations.
//!
//! Implements per-element address computation and memory access for all vector
//! load/store variants: unit-stride, strided, indexed, mask, whole-register,
//! and fault-only-first. All accesses go through the CPU's address translation
//! and bus interface.

use crate::common::{AccessType, Trap, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::signals::VectorOp;
use crate::core::units::vpu::types::{ElemIdx, Sew, VRegIdx, parse_vtype};

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
    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            let val = mem_read_element(cpu, addr, eew)?;
            let dest = VRegIdx::new(vd.as_u8() + seg as u8);
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

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let val = mem_read_element(cpu, addr, eew)?;
            let dest = VRegIdx::new(vd.as_u8() + seg as u8);
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

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let offset = cpu.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let val = mem_read_element(cpu, addr, data_sew)?;
            let dest = VRegIdx::new(vd.as_u8() + seg as u8);
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

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        for seg in 0..nf {
            let addr = base.wrapping_add(((i * nf + seg) as u64).wrapping_mul(eew_bytes));
            let src = VRegIdx::new(vs3.as_u8() + seg as u8);
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

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let elem_base = base.wrapping_add((i as i64).wrapping_mul(stride) as u64);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(eew_bytes));
            let src = VRegIdx::new(vs3.as_u8() + seg as u8);
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

    for i in vstart..vl {
        if !is_element_active(cpu, i, vm) {
            continue;
        }
        let offset = cpu.regs.vpr().read_element(vs2, ElemIdx::new(i), idx_eew);
        let elem_base = base.wrapping_add(offset);
        for seg in 0..nf {
            let addr = elem_base.wrapping_add((seg as u64).wrapping_mul(data_bytes));
            let src = VRegIdx::new(vs3.as_u8() + seg as u8);
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

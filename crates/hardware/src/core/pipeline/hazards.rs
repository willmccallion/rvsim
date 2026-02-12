//! Data Hazard Detection and Forwarding.
//!
//! This module implements the logic for maintaining pipeline consistency when data
//! dependencies exist between instructions. It provides:
//! 1. **Hazard Detection:** Identifies load-use hazards that require pipeline stalls.
//! 2. **Operand Forwarding:** Resolves Read-After-Write (RAW) hazards by bypassing the register file.
//! 3. **Superscalar Support:** Handles intra-bundle dependencies in wide-issue configurations.

use crate::core::pipeline::latches::{ExMem, ExMemEntry};
use crate::core::pipeline::latches::{IdEx, IdExEntry, IfId, MemWb};

/// Checks if a pipeline stall is needed due to a load-use data hazard.
///
/// A load-use hazard occurs when an instruction in the Decode (ID) stage depends on
/// data that is currently being loaded from memory by an instruction in the Execute (EX) stage.
///
/// # Arguments
///
/// * `id_ex` - The ID/EX pipeline latch containing instructions currently in execution.
/// * `if_id` - The IF/ID pipeline latch containing instructions being decoded.
///
/// # Returns
///
/// `true` if a stall is required to resolve the load-use hazard, `false` otherwise.
///
/// # Examples
///
/// ```ignore
/// use hardware::core::pipeline::hazards::need_stall_load_use;
/// use hardware::core::pipeline::latches::{IdEx, IfId};
///
/// // Example scenario:
/// // ID/EX stage: lw x1, 0(x2)    <- loads into x1 (in execute)
/// // IF/ID stage: add x3, x1, x4  <- uses x1 (in decode)
/// //
/// // This creates a load-use hazard requiring a 1-cycle stall
///
/// let stall_needed = need_stall_load_use(&id_ex, &if_id);
/// if stall_needed {
///     // Insert bubble (nop) into ID/EX stage
///     // Repeat IF/ID stage instruction fetch
/// }
/// ```
pub fn need_stall_load_use(id_ex: &IdEx, if_id: &IfId) -> bool {
    for ex_inst in &id_ex.entries {
        if !ex_inst.ctrl.mem_read {
            continue;
        }

        if !ex_inst.ctrl.fp_reg_write && ex_inst.rd == 0 {
            continue;
        }

        for id_inst in &if_id.entries {
            let inst = id_inst.inst;
            let next_rs1 = ((inst >> 15) & 0x1f) as usize;
            let next_rs2 = ((inst >> 20) & 0x1f) as usize;
            let next_rs3 = ((inst >> 27) & 0x1f) as usize;

            if ex_inst.rd == next_rs1 || ex_inst.rd == next_rs2 || ex_inst.rd == next_rs3 {
                return true;
            }
        }
    }
    false
}

/// Forwards register values from later pipeline stages to resolve data hazards.
///
/// This function implements the register forwarding (bypassing) logic. It prioritizes the
/// most recent results from the Execute, Memory, or Writeback stages to satisfy the
/// source register requirements of an instruction in the Decode stage.
///
/// # Arguments
///
/// * `id_entry` - The ID/EX entry requiring forwarded register values.
/// * `ex_mem` - The EX/MEM pipeline latch containing one-cycle-old results.
/// * `mem_wb_old` - The MEM/WB pipeline latch containing two-cycle-old results.
/// * `mem_wb_fresh` - The fresh MEM/WB pipeline latch containing current cycle results.
/// * `current_ex_results` - Results from instructions in the current execute stage cycle.
/// * `trace` - Boolean flag to enable or disable forwarding trace messages.
///
/// # Returns
///
/// A tuple `(rs1_val, rs2_val, rs3_val)` containing the most recent available register values.
pub fn forward_rs(
    id_entry: &IdExEntry,
    ex_mem: &ExMem,
    mem_wb_old: &MemWb,
    mem_wb_fresh: &MemWb,
    current_ex_results: &[ExMemEntry],
    trace: bool,
) -> (u64, u64, u64) {
    let mut a = id_entry.rv1;
    let mut b = id_entry.rv2;
    let mut c = id_entry.rv3;

    let mut a_src = "RegFile";
    let mut b_src = "RegFile";
    let mut c_src = "RegFile";

    let check = |dest: usize, dest_fp: bool, src: usize, src_fp: bool| -> bool {
        if dest_fp != src_fp {
            return false;
        }
        if dest != src {
            return false;
        }
        if !dest_fp && dest == 0 {
            return false;
        }
        true
    };

    for wb_entry in mem_wb_old.entries.iter() {
        if wb_entry.trap.is_some() {
            continue;
        }
        if wb_entry.ctrl.reg_write || wb_entry.ctrl.fp_reg_write {
            let wb_val = if wb_entry.ctrl.mem_read {
                wb_entry.load_data
            } else if wb_entry.ctrl.jump {
                wb_entry.pc.wrapping_add(wb_entry.inst_size)
            } else {
                wb_entry.alu
            };

            let dest_fp = wb_entry.ctrl.fp_reg_write;

            if check(wb_entry.rd, dest_fp, id_entry.rs1, id_entry.ctrl.rs1_fp) {
                a = wb_val;
                a_src = "WB_Latch";
            }
            if check(wb_entry.rd, dest_fp, id_entry.rs2, id_entry.ctrl.rs2_fp) {
                b = wb_val;
                b_src = "WB_Latch";
            }
            if check(wb_entry.rd, dest_fp, id_entry.rs3, id_entry.ctrl.rs3_fp) {
                c = wb_val;
                c_src = "WB_Latch";
            }
        }
    }

    for wb_entry in mem_wb_fresh.entries.iter() {
        if wb_entry.trap.is_some() {
            continue;
        }
        if wb_entry.ctrl.reg_write || wb_entry.ctrl.fp_reg_write {
            let val = if wb_entry.ctrl.mem_read {
                wb_entry.load_data
            } else if wb_entry.ctrl.jump {
                wb_entry.pc.wrapping_add(wb_entry.inst_size)
            } else {
                wb_entry.alu
            };

            let dest_fp = wb_entry.ctrl.fp_reg_write;

            if check(wb_entry.rd, dest_fp, id_entry.rs1, id_entry.ctrl.rs1_fp) {
                if trace {
                    eprintln!(
                        "[Forward] PC={:#x} rs1=x{} Val={:#x} Source=MEM_WB_Fresh (Prev: {})",
                        id_entry.pc, id_entry.rs1, val, a_src
                    );
                }
                a = val;
                a_src = "MEM_WB_Fresh";
            }
            if check(wb_entry.rd, dest_fp, id_entry.rs2, id_entry.ctrl.rs2_fp) {
                if trace {
                    eprintln!(
                        "[Forward] PC={:#x} rs2=x{} Val={:#x} Source=MEM_WB_Fresh (Prev: {})",
                        id_entry.pc, id_entry.rs2, val, b_src
                    );
                }
                b = val;
                b_src = "MEM_WB_Fresh";
            }
            if check(wb_entry.rd, dest_fp, id_entry.rs3, id_entry.ctrl.rs3_fp) {
                c = val;
                let _ = c_src;
            }
        }
    }

    for mem_entry in ex_mem.entries.iter() {
        if mem_entry.trap.is_some() {
            continue;
        }
        if (mem_entry.ctrl.reg_write || mem_entry.ctrl.fp_reg_write) && !mem_entry.ctrl.mem_read {
            let ex_val = if mem_entry.ctrl.jump {
                mem_entry.pc.wrapping_add(mem_entry.inst_size)
            } else {
                mem_entry.alu
            };

            let dest_fp = mem_entry.ctrl.fp_reg_write;

            if check(mem_entry.rd, dest_fp, id_entry.rs1, id_entry.ctrl.rs1_fp) {
                if trace {
                    eprintln!(
                        "[Forward] PC={:#x} rs1=x{} Val={:#x} Source=EX_MEM (Prev: {})",
                        id_entry.pc, id_entry.rs1, ex_val, a_src
                    );
                }
                a = ex_val;
                a_src = "EX_MEM";
            }
            if check(mem_entry.rd, dest_fp, id_entry.rs2, id_entry.ctrl.rs2_fp) {
                if trace {
                    eprintln!(
                        "[Forward] PC={:#x} rs2=x{} Val={:#x} Source=EX_MEM (Prev: {})",
                        id_entry.pc, id_entry.rs2, ex_val, b_src
                    );
                }
                b = ex_val;
                b_src = "EX_MEM";
            }
            if check(mem_entry.rd, dest_fp, id_entry.rs3, id_entry.ctrl.rs3_fp) {
                c = ex_val;
                c_src = "EX_MEM";
            }
        }
    }

    for ex_entry in current_ex_results.iter().rev() {
        if ex_entry.trap.is_some() {
            continue;
        }

        if (ex_entry.ctrl.reg_write || ex_entry.ctrl.fp_reg_write) && !ex_entry.ctrl.mem_read {
            let ex_val = if ex_entry.ctrl.jump {
                ex_entry.pc.wrapping_add(ex_entry.inst_size)
            } else {
                ex_entry.alu
            };

            let dest_fp = ex_entry.ctrl.fp_reg_write;

            if check(ex_entry.rd, dest_fp, id_entry.rs1, id_entry.ctrl.rs1_fp) {
                if trace {
                    eprintln!(
                        "[Forward] PC={:#x} rs1=x{} Val={:#x} Source=Intra-Bundle (Prev: {})",
                        id_entry.pc, id_entry.rs1, ex_val, a_src
                    );
                }
                a = ex_val;
                a_src = "Intra-Bundle";
            }
            if check(ex_entry.rd, dest_fp, id_entry.rs2, id_entry.ctrl.rs2_fp) {
                if trace {
                    eprintln!(
                        "[Forward] PC={:#x} rs2=x{} Val={:#x} Source=Intra-Bundle (Prev: {})",
                        id_entry.pc, id_entry.rs2, ex_val, b_src
                    );
                }
                b = ex_val;
                b_src = "Intra-Bundle";
            }
            if check(ex_entry.rd, dest_fp, id_entry.rs3, id_entry.ctrl.rs3_fp) {
                c = ex_val;
                let _ = c_src;
            }
        }
    }

    let _ = a_src;
    let _ = b_src;
    (a, b, c)
}

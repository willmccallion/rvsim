//! Rename Stage: ROB allocation, store buffer allocation, scoreboard/PRF marking.
//!
//! For the in-order backend: uses scoreboard to track register producers.
//! For the O3 backend (has_prf = true): uses physical register file, free list,
//! and rename map to implement full register renaming.
//!
//! Source register tags are captured BEFORE the scoreboard is updated for rd,
//! so that instructions reading their own destination (e.g. ADDI x5, x5, 16)
//! get the previous producer's tag, not their own.

use crate::core::Cpu;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;

/// Executes the rename stage: allocate ROB/SB entries, capture source tags, mark scoreboard.
pub fn rename_stage<E: ExecutionEngine>(
    cpu: &mut Cpu,
    input: &mut Vec<IdExEntry>,
    engine: &mut E,
    rename_output: &mut Vec<RenameIssueEntry>,
) {
    let entries = std::mem::take(input);

    // Compute the dispatch budget once. can_accept() returns
    // min(rob_free, sb_free, iq_free, width). During the loop below,
    // ROB and SB allocations update their own free counts, but the
    // issue queue free count does NOT change (entries go into
    // rename_output, not the IQ). Without a budget counter, every
    // iteration sees the same iq_free and can over-allocate, causing
    // dispatch failures in the next backend tick.
    let mut budget = engine.can_accept();

    for id in entries {
        // Check if engine can accept more instructions
        if budget == 0 {
            // Put unconsumed entries back
            input.push(id);
            continue;
        }

        if engine.has_prf() {
            // ── O3 backend: full physical register renaming ────────────────
            // Capture source physical regs BEFORE updating rename map for rd
            let rs1_phys = engine.rename_map().get(id.rs1, id.ctrl.rs1_fp);
            let rs2_phys = engine.rename_map().get(id.rs2, id.ctrl.rs2_fp);
            let rs3_phys = if id.ctrl.rs3_fp {
                engine.rename_map().get(id.rs3, true)
            } else {
                PhysReg(0)
            };

            // Allocate destination physical register
            // Skip x0 for integer writes — x0 is hardwired zero and must not
            // consume a physical register (it would never be freed at commit).
            let needs_dst = (id.ctrl.reg_write && id.rd != 0) || id.ctrl.fp_reg_write;
            let (rd_phys, old_phys_dst) = if needs_dst {
                let new_p = match engine.free_list_mut().allocate() {
                    Some(p) => p,
                    None => {
                        // Free list empty (shouldn't happen if can_accept accounts for it)
                        input.push(id);
                        break;
                    }
                };
                let old_p = engine.rename_map().get(id.rd, id.ctrl.fp_reg_write);
                (new_p, old_p)
            } else {
                (PhysReg(0), PhysReg(0))
            };

            // Allocate ROB entry
            let rob_tag = match engine.rob_mut().allocate(
                id.pc,
                id.inst,
                id.inst_size,
                id.rd,
                id.ctrl.fp_reg_write,
                id.ctrl,
                rd_phys,
                old_phys_dst,
            ) {
                Some(tag) => tag,
                None => {
                    // ROB full — reclaim the physical reg we just allocated
                    if needs_dst {
                        engine.free_list_mut().reclaim(rd_phys);
                    }
                    input.push(id);
                    break;
                }
            };

            // Update speculative rename map and mark PRF not-ready
            if needs_dst {
                engine
                    .rename_map_mut()
                    .set(id.rd, id.ctrl.fp_reg_write, rd_phys);
                engine.prf_mut().allocate(rd_phys);
            }

            // Allocate store buffer entry if this is a store
            if id.ctrl.mem_write {
                let width = id.ctrl.width;
                if !engine.store_buffer_mut().allocate(rob_tag, width) {
                    input.push(id);
                    break;
                }
            }

            // Allocate load queue entry if this is a load
            if id.ctrl.mem_read
                && let Some(lq) = engine.load_queue_mut()
            {
                let width = id.ctrl.width;
                if !lq.allocate(rob_tag, width) {
                    input.push(id);
                    break;
                }
            }

            // Build RenameIssueEntry with physical register identifiers
            // rs*_tag fields carry the physical reg as a packed tag for the IQ
            // (the IQ will look them up in the PRF at dispatch time)
            let entry = RenameIssueEntry {
                rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rs1: id.rs1,
                rs2: id.rs2,
                rs3: id.rs3,
                rd: id.rd,
                imm: id.imm,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                // Store physical regs in the rs*_phys fields
                rs1_phys,
                rs2_phys,
                rs3_phys,
                rd_phys,
                // Legacy tag fields unused with PRF
                rs1_tag: None,
                rs2_tag: None,
                rs3_tag: None,
                ctrl: id.ctrl,
                trap: id.trap,
                exception_stage: id.exception_stage,
                pred_taken: id.pred_taken,
                pred_target: id.pred_target,
                ghr_snapshot: id.ghr_snapshot,
                ras_snapshot: id.ras_snapshot,
            };

            if cpu.trace {
                eprintln!(
                    "RN  pc={:#x} rob_tag={} rd_phys=p{} old=p{}",
                    entry.pc, entry.rob_tag.0, rd_phys.0, old_phys_dst.0
                );
            }

            rename_output.push(entry);
            budget -= 1;
        } else {
            // ── In-order / legacy backend: scoreboard-based rename ─────────
            // Allocate ROB entry
            let rob_tag = match engine.rob_mut().allocate(
                id.pc,
                id.inst,
                id.inst_size,
                id.rd,
                id.ctrl.fp_reg_write,
                id.ctrl,
                PhysReg(0),
                PhysReg(0),
            ) {
                Some(tag) => tag,
                None => {
                    // ROB full, stall
                    input.push(id);
                    break;
                }
            };

            // Capture source register tags BEFORE updating scoreboard for rd.
            let rs1_tag = engine.scoreboard().get_producer(id.rs1, id.ctrl.rs1_fp);
            let rs2_tag = engine.scoreboard().get_producer(id.rs2, id.ctrl.rs2_fp);
            let rs3_tag = if id.ctrl.rs3_fp {
                engine.scoreboard().get_producer(id.rs3, true)
            } else {
                None
            };

            // Mark scoreboard: this instruction will write rd
            if id.ctrl.reg_write || id.ctrl.fp_reg_write {
                engine
                    .scoreboard_mut()
                    .set_producer(id.rd, id.ctrl.fp_reg_write, rob_tag);
            }

            // Allocate store buffer entry if this is a store
            if id.ctrl.mem_write {
                let width = id.ctrl.width;
                if !engine.store_buffer_mut().allocate(rob_tag, width) {
                    input.push(id);
                    break;
                }
            }

            // Create RenameIssueEntry — operand values are 0, read at issue stage
            let entry = RenameIssueEntry {
                rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rs1: id.rs1,
                rs2: id.rs2,
                rs3: id.rs3,
                rd: id.rd,
                imm: id.imm,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                rs1_phys: PhysReg(0),
                rs2_phys: PhysReg(0),
                rs3_phys: PhysReg(0),
                rd_phys: PhysReg(0),
                rs1_tag,
                rs2_tag,
                rs3_tag,
                ctrl: id.ctrl,
                trap: id.trap,
                exception_stage: id.exception_stage,
                pred_taken: id.pred_taken,
                pred_target: id.pred_target,
                ghr_snapshot: id.ghr_snapshot,
                ras_snapshot: id.ras_snapshot,
            };

            if cpu.trace {
                eprintln!("RN  pc={:#x} rob_tag={}", entry.pc, entry.rob_tag.0);
            }

            rename_output.push(entry);
            budget -= 1;
        }
    }
}

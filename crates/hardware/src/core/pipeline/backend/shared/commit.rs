//! Commit Stage: retire instructions from ROB head.
//!
//! This stage retires the oldest instruction(s) from the ROB in program order:
//! 1. Write results to the register file.
//! 2. Apply deferred CSR writes.
//! 3. Mark store buffer entries as Committed.
//! 4. Handle traps/interrupts.
//! 5. Drain one committed store to memory per cycle.

use crate::common::constants::{
    DELEG_MEIP_BIT, DELEG_MSIP_BIT, DELEG_MTIP_BIT, DELEG_SEIP_BIT, DELEG_SSIP_BIT, DELEG_STIP_BIT,
};
use crate::common::constants::{PAGE_SHIFT, VPN_MASK};
use crate::common::{Asid, LrScRecord, PhysAddr, RegIdx, SfenceVmaInfo, Trap, Vpn};
use crate::sim::CoreCtx;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::arch::trap::TrapHandler;
use crate::sim::per_hart_debug::PC_TRACE_MAX;
use crate::core::pipeline::checkpoint::CheckpointTable;
use crate::core::pipeline::engine::BackendCommon;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::outstanding::OutstandingStore;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::{Rob, RobState};
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::signals::{AluOp, ControlFlow, MemWidth, SystemOp, VectorOp};
use crate::core::pipeline::store_buffer::{StoreBuffer, StoreResolution, width_to_bytes};
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::vpu::types::{VRegIdx, VecPhysReg};
use crate::sim::components::ComponentId;
use crate::sim::packet::{AccessSize, MemOp, Packet, WriteData};
use crate::sim::stats::paths;
use crate::trace_branch;
use crate::trace_commit;
use crate::trace_csr;
use crate::trace_trap;

/// Executes the Commit stage.
///
/// Retires up to `width` instructions from the ROB head per cycle.
/// Handles register writes, CSR application, trap dispatch, and
/// store buffer drain. Store drains emit `MemReq` packets through the
/// engine's `BackendCommon`.
#[allow(clippy::too_many_arguments)]
pub fn commit_stage(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    rob: &mut Rob,
    store_buffer: &mut StoreBuffer,
    scoreboard: &mut Scoreboard,
    committed_rename_map: &mut RenameMap,
    free_list: &mut FreeList<PhysReg>,
    width: usize,
    mut load_queue: Option<&mut LoadQueue>,
    mut prf: Option<&mut PhysRegFile>,
    mut checkpoints: Option<&mut CheckpointTable>,
    mut vec_prf: Option<&mut VecPhysRegFile>,
    mut vec_free_list: Option<&mut FreeList<VecPhysReg>>,
    mut vec_store_buffer: Option<&mut crate::core::pipeline::vec_store_buffer::VecStoreBuffer>,
    redirect_pending: &mut bool,
) -> Option<(Trap, u64)> {
    let mut trap_event: Option<(Trap, u64)> = None;

    // Always check, even with empty ROB (timer firing during a stall).
    {
        let epc = if state.hart.wfi_waiting {
            state.hart.wfi_pc
        } else if let Some(head) = rob.peek_head() {
            head.pc
        } else {
            state.hart.committed_next_pc
        };

        let interrupt = check_interrupts(state);
        if let Some(interrupt_trap) = interrupt {
            state.hart.wfi_waiting = false;
            trace_trap!(state.config.general.trace_instructions;
                event      = "interrupt",
                epc        = %crate::trace::Hex(epc),
                cause      = ?interrupt_trap,
                mip        = %crate::trace::Hex(state.hart.csrs.mip),
                mie        = %crate::trace::Hex(state.hart.csrs.mie),
                mstatus    = %crate::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: interrupt detected — flushing pipeline"
            );
            trap_event = Some((interrupt_trap, epc));
        } else if state.hart.wfi_waiting {
            // Block commit while WFI is active so wrong-path post-WFI ops can't retire.
            let pending = state.hart.csrs.mip;
            let enabled = state.hart.csrs.mie;
            if (pending & enabled) != 0 {
                state.hart.wfi_waiting = false;
                state.hart.pc = state.hart.wfi_pc;
                *redirect_pending = true;
            } else {
                state.stats.counter(paths::core::pipeline::CYCLES_WFI).inc();
            }
            state.stats.counter(paths::core::commit::RETIRE_HIST_ZERO).inc();
            return trap_event;
        }
    }

    if trap_event.is_some() {
        state.stats.counter(paths::core::commit::RETIRE_HIST_ZERO).inc();
        return trap_event;
    }

    let mut retired_count: usize = 0;
    let rob_empty_at_start = rob.peek_head().is_none();
    for _ in 0..width {
        let Some(head) = rob.peek_head() else { break };

        // Block load retirement while older stores have unresolved addresses,
        // so memory2 can still flag a violation against a later-resolving store.
        if head.state == RobState::Completed
            && head.ctrl.mem_read
            && store_buffer.has_unresolved_store_before(head.tag)
        {
            break;
        }

        if head.state == RobState::Issued {
            break;
        }

        if head.state == RobState::Faulted {
            if let Some(entry) = rob.commit_head()
                && let Some(ref the_trap) = entry.trap
            {
                #[cfg(feature = "commit-log")]
                if let Some(ref mut log) = state.commit_log {
                    use crate::common::Trap;
                    use std::io::Write;
                    // Spike skips fetch-stage page/access faults (no valid bits).
                    let skip = matches!(
                        the_trap,
                        Trap::InstructionPageFault(_)
                            | Trap::InstructionAccessFault(_)
                            | Trap::InstructionAddressMisaligned(_)
                    );
                    if !skip {
                        let _ =
                            writeln!(log, "core   0: 0x{:016x} (0x{:08x})", entry.pc, entry.inst);
                    }
                }
                trace_trap!(state.config.general.trace_instructions;
                    event     = "sync-exception",
                    pc        = %crate::trace::Hex(entry.pc),
                    rob_tag   = entry.tag.0,
                    cause     = ?the_trap,
                    priv_mode = ?state.hart.privilege,
                    mstatus   = %crate::trace::Hex(state.hart.csrs.mstatus),
                    "CM: synchronous exception at commit"
                );
                // Faulting entry was popped before the post-trap flush, so reclaim its phys_dst here.
                if entry.phys_dst.0 != 0 {
                    free_list.reclaim(entry.phys_dst);
                }
                if let Some(ref mut vfl) = vec_free_list {
                    for i in 0..entry.vec_dst_count as usize {
                        if !entry.vec_phys_dst[i].is_zero() {
                            vfl.reclaim(entry.vec_phys_dst[i]);
                        }
                    }
                }
                trap_event = Some((the_trap.clone(), entry.pc));
            }
            break;
        }

        // SFENCE.VMA must wait for committed stores to reach RAM so PTW sees current PTEs.
        if head.ctrl.system_op == SystemOp::SfenceVma && store_buffer.has_committed_stores() {
            break;
        }
        // CBO ops (Zicboz / Zicbom) drain prior committed stores first so
        // memory ordering against earlier writes matches a normal store and
        // any stale PTE in the SB has settled before we re-translate.
        if matches!(
            head.ctrl.system_op,
            SystemOp::CboZero | SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush
        ) && store_buffer.has_committed_stores()
        {
            break;
        }

        let Some(entry) = rob.commit_head() else { break };
        retired_count += 1;

        // For taken branches/jumps, committed_next_pc must be the target so interrupt EPC is correct.
        state.hart.committed_next_pc = match entry.ctrl.control_flow {
            ControlFlow::Jump => {
                entry.bp_target.unwrap_or_else(|| entry.pc.wrapping_add(entry.inst_size.as_u64()))
            }
            ControlFlow::Branch if entry.bp_outcome.taken => {
                entry.bp_target.unwrap_or_else(|| entry.pc.wrapping_add(entry.inst_size.as_u64()))
            }
            _ => entry.pc.wrapping_add(entry.inst_size.as_u64()),
        };

        trace_commit!(state.config.general.trace_instructions;
            rob_tag    = entry.tag.0,
            pc         = %crate::trace::Hex(entry.pc),
            rd         = entry.rd.as_usize(),
            rd_phys    = entry.phys_dst.0,
            old_phys   = entry.old_phys_dst.0,
            result     = %crate::trace::Hex(entry.result.unwrap_or(0)),
            is_fp      = entry.ctrl.fp_reg_write,
            reg_write  = entry.ctrl.reg_write,
            is_store   = entry.ctrl.mem_write,
            is_load    = entry.ctrl.mem_read,
            fp_flags   = entry.fp_flags,
            "CM: instruction retired"
        );

        // Defer commit log write until after the register write so rd value is available.
        #[cfg(feature = "commit-log")]
        let commit_log_entry: Option<(u64, u32, bool, usize, u64)> = {
            if state.commit_log.is_some() {
                let has_rd =
                    (entry.ctrl.reg_write && !entry.rd.is_zero()) || entry.ctrl.fp_reg_write;
                Some((entry.pc, entry.inst, has_rd, entry.rd.as_usize(), entry.result.unwrap_or(0)))
            } else {
                None
            }
        };

        let hart_idx = state.hart.hart_id.as_index();
        let pc_trace = &mut state.per_hart_debug[hart_idx].pc_trace;
        pc_trace.push((entry.pc, entry.inst));
        if pc_trace.len() > PC_TRACE_MAX {
            let _ = pc_trace.remove(0);
        }

        if entry.inst != 0 && entry.inst != 0x13 {
            state.hart.instructions_retired += 1;
            state.stats.counter(paths::hart::RETIRED_INSTS).inc();
            update_instruction_stats(state, &entry);
        }

        if entry.bp_update {
            state.core.branch_predictor.update_branch(
                entry.bp_pc,
                entry.bp_outcome.taken,
                entry.bp_target,
                &entry.bp_ghr_snapshot,
            );
            trace_branch!(state.config.general.trace_instructions;
                event         = "update",
                pc            = %crate::trace::Hex(entry.bp_pc),
                rob_tag       = entry.tag.0,
                actual_taken  = entry.bp_outcome.taken,
                actual_target = %crate::trace::Hex(entry.bp_target.unwrap_or(0)),
                mispredicted  = entry.bp_outcome.mispredicted,
                "CM: branch predictor updated at commit"
            );
            if entry.bp_outcome.mispredicted {
                state.stats.counter(paths::core::bp::COMMITTED_MISPREDICTS).inc();
            } else {
                state.stats.counter(paths::core::bp::COMMITTED_HITS).inc();
            }
        }

        debug_assert!(
            entry.result.is_some() || (!entry.ctrl.reg_write && !entry.ctrl.fp_reg_write),
            "CM: committing instruction with reg_write but no result: rob_tag={} pc={:#x}",
            entry.tag.0,
            entry.pc,
        );
        let val = entry.result.unwrap_or(0);
        if entry.ctrl.fp_reg_write {
            state.hart.regs.write_f(entry.rd, val);
            scoreboard.clear_if_match(entry.rd, true, entry.tag);
            if entry.old_phys_dst.0 != entry.phys_dst.0 {
                free_list.reclaim(entry.old_phys_dst);
            }
            committed_rename_map.set(entry.rd, true, entry.phys_dst);
            state.hart.csrs.mstatus = (state.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            state.hart.csrs.sstatus = (state.hart.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            trace_commit!(state.config.general.trace_instructions;
                pc       = %crate::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                reg      = entry.rd.as_usize(),
                rd_phys  = entry.phys_dst.0,
                old_phys = entry.old_phys_dst.0,
                value    = %crate::trace::Hex(val),
                is_fp    = true,
                "CM: FP register write"
            );
        } else if entry.ctrl.reg_write && !entry.rd.is_zero() {
            state.hart.regs.write(entry.rd, val);
            scoreboard.clear_if_match(entry.rd, false, entry.tag);
            if entry.old_phys_dst.0 != entry.phys_dst.0 {
                free_list.reclaim(entry.old_phys_dst);
            }
            committed_rename_map.set(entry.rd, false, entry.phys_dst);
            trace_commit!(state.config.general.trace_instructions;
                pc       = %crate::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                reg      = entry.rd.as_usize(),
                rd_phys  = entry.phys_dst.0,
                old_phys = entry.old_phys_dst.0,
                value    = %crate::trace::Hex(val),
                is_fp    = false,
                "CM: integer register write"
            );
        }

        if entry.vec_dst_count > 0 {
            let vd_base = entry.ctrl.vd.as_u8();
            for i in 0..entry.vec_dst_count as usize {
                let vreg = VRegIdx::new(vd_base + i as u8);
                if let Some(ref mut vprf) = vec_prf {
                    let bytes = vprf.read_bytes(entry.vec_phys_dst[i]);
                    state.hart.regs.vpr_mut().write_bytes(vreg, bytes);
                }
                if let Some(ref mut vfl) = vec_free_list
                    && entry.vec_old_phys_dst[i] != entry.vec_phys_dst[i]
                {
                    vfl.reclaim(entry.vec_old_phys_dst[i]);
                }
                committed_rename_map.set_vec(vreg, entry.vec_phys_dst[i]);
                scoreboard.clear_vec_if_match(vreg, entry.tag);
            }
            state.hart.csrs.mstatus = (state.hart.csrs.mstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
            state.hart.csrs.sstatus = (state.hart.csrs.sstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
            state.hart.csrs.vstart = 0;
        }

        #[cfg(feature = "commit-log")]
        if let Some((pc, inst, has_rd, rd, val)) = commit_log_entry
            && let Some(ref mut log) = state.commit_log
        {
            use std::io::Write;
            if has_rd {
                let _ = writeln!(log, "core   0: 0x{pc:016x} (0x{inst:08x}) x{rd} 0x{val:016x}");
            } else {
                let _ = writeln!(log, "core   0: 0x{pc:016x} (0x{inst:08x})");
            }
        }

        if let Some(pte_upd) = entry.pte_update {
            write_store_to_memory(state, common, pte_upd.pte_addr, pte_upd.pte_value, MemWidth::Double);
        }

        // Apply fp_flags before CSR writes to keep execute-time CSR reads of fflags consistent.
        if entry.fp_flags != 0 {
            state.hart.csrs.fflags |= entry.fp_flags as u64;
            state.hart.csrs.mstatus = (state.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            state.hart.csrs.sstatus = (state.hart.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
        }

        if entry.vxsat {
            state.hart.csrs.vxsat = 1;
        }

        if let Some(csr_update) = entry.csr_update {
            // SATP write: drain SB so PTW reads up-to-date PTEs after translation mode change.
            if csr_update.addr == csr::SATP {
                drain_all_committed(state, common, store_buffer, vec_store_buffer.as_deref_mut());
            }
            // O3 applies fflags/fcsr eagerly at complete time; don't re-apply.
            if !csr_update.applied {
                state.csr_write(csr_update.addr, csr_update.new_val);
            }
            trace_csr!(state.config.general.trace_instructions;
                op       = if csr_update.applied { "write-eager" } else { "write-deferred" },
                pc       = %crate::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                csr_addr = %crate::trace::Hex32(csr_update.addr.as_u32()),
                old_val  = %crate::trace::Hex(csr_update.old_val),
                new_val  = %crate::trace::Hex(csr_update.new_val),
                deferred = !csr_update.applied,
                "CM: CSR write applied at commit"
            );
            // SATP redirect: post-execute fetches used old tables; reset state.hart.pc to next inst.
            if csr_update.addr == csr::SATP {
                state.hart.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
                *redirect_pending = true;
            }
            break;
        }

        if entry.ctrl.system_op == SystemOp::Mret {
            state.do_mret();
            state.hart.committed_next_pc = state.hart.pc;
            trace_trap!(state.config.general.trace_instructions;
                event      = "return",
                insn       = "MRET",
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::trace::Hex(state.hart.pc),
                mstatus    = %crate::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: MRET committed — privilege restored"
            );
            break;
        }
        if entry.ctrl.system_op == SystemOp::Sret {
            state.do_sret();
            state.hart.committed_next_pc = state.hart.pc;
            trace_trap!(state.config.general.trace_instructions;
                event      = "return",
                insn       = "SRET",
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::trace::Hex(state.hart.pc),
                mstatus    = %crate::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: SRET committed — privilege restored"
            );
            break;
        }

        if entry.ctrl.system_op == SystemOp::Wfi {
            if state.hart.csrs.mie != 0 || state.hart.csrs.mip != 0 {
                state.hart.wfi_waiting = true;
                state.hart.wfi_pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            } else {
                // Nothing enabled or pending — treat as NOP to avoid OpenSBI early-boot deadlock.
                state.hart.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
                *redirect_pending = true;
            }
            state.hart.committed_next_pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            break;
        }

        // LR/SC reservation checks are deferred to commit so squashed insts can't corrupt them.
        if let Some(lr_sc_rec) = entry.lr_sc {
            match lr_sc_rec {
                LrScRecord::Lr { paddr } => {
                    state.set_reservation(paddr);
                }
                LrScRecord::Sc { paddr } => {
                    if state.check_reservation(paddr) {
                        state.clear_reservation();
                    } else {
                        // SC failure: undo Memory2's optimistic success (rd=0) and re-fetch.
                        store_buffer.cancel(entry.tag);
                        if entry.ctrl.reg_write && !entry.rd.is_zero() {
                            state.hart.regs.write(entry.rd, 1);
                            // Patch PRF too so post-flush rename sees rd=1, not optimistic 0.
                            if let Some(ref mut prf) = prf {
                                prf.write(entry.phys_dst, 1);
                            }
                        }
                        state.hart.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
                        *redirect_pending = true;
                        break;
                    }
                }
            }
        }

        if entry.ctrl.mem_write {
            if let Some(paddr) = store_buffer.find_paddr(entry.tag) {
                // RISC-V §8.2: a non-LR/SC store to the reservation set must
                // fail any paired SC, and any store breaks other harts'
                // reservations on the line.
                if entry.lr_sc.is_none() && state.check_reservation(paddr) {
                    state.clear_reservation();
                }
                state.invalidate_other_reservations(paddr);
            }
            store_buffer.mark_committed(entry.tag);
        } else if crate::core::units::vpu::mem::is_vec_store(entry.ctrl.vec_op) {
            // Vector store data lives in the dedicated VecStoreBuffer.
            store_buffer.mark_committed(entry.tag);
            if let Some(vsb) = vec_store_buffer.as_deref_mut() {
                vsb.mark_committed(entry.tag);
            }
        }

        if entry.ctrl.mem_read
            && let Some(ref mut lq) = load_queue
        {
            lq.deallocate(entry.tag);
        } else if crate::core::units::vpu::mem::is_vec_load(entry.ctrl.vec_op)
            && let Some(ref mut lq) = load_queue
        {
            // Per-element micro-op slots leak otherwise; vec loads stay parked in IQ.
            lq.deallocate(entry.tag);
        }

        if let Some(ckpt_id) = entry.checkpoint_id
            && let Some(ref mut ckpt_table) = checkpoints
        {
            ckpt_table.free(ckpt_id);
        }

        if entry.ctrl.system_op == SystemOp::FenceI {
            drain_all_committed(state, common, store_buffer, vec_store_buffer.as_deref_mut());
            // I-cache flush after drain so refills see new data; force a fresh redirect.
            let _ = state.core.l1_i_cache.invalidate_all();
            state.hart.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            *redirect_pending = true;
            // FENCE.I serializes: break so younger insts fetched pre-drain don't retire here.
            break;
        } else if entry.ctrl.system_op == SystemOp::Fence {
            let pred_bits = ((entry.inst >> 24) & 0xF) as u8;
            let pred_w = pred_bits & 0b0001 != 0;
            let pred_r = pred_bits & 0b0010 != 0;
            // pred.w drains the SB; pred.r is satisfied by commit order.
            if pred_w || pred_r {
                drain_all_committed(state, common, store_buffer, vec_store_buffer.as_deref_mut());
            }
        }

        // SFENCE.VMA: SB is empty (stall above). Flush TLBs, clear reservation, full squash.
        if let Some(info) = entry.sfence_vma {
            sfence_vma_commit(state, &info);
            state.clear_reservation();
            state.hart.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            *redirect_pending = true;
            break;
        }

        // CBO ops (Zicboz / Zicbom): SB is empty (stall above). entry.result
        // holds rs1 from execute. Resolve the gate + translation here so a
        // fault routes through the standard commit-time trap path and any
        // freshly-committed PTE writes are visible to the walk.
        if matches!(
            entry.ctrl.system_op,
            SystemOp::CboZero | SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush
        ) {
            let rs1 = entry.result.unwrap_or(0);
            if let Some(trap) = commit_cbo(state, common, entry.ctrl.system_op, rs1, entry.inst) {
                state.trap(&trap, entry.pc);
                break;
            }
        }

        state.hart.regs.write(RegIdx::new(0), 0);
    }

    if retired_count == 0 && rob_empty_at_start {
        state.stats.counter(paths::core::pipeline::CYCLES_ROB_EMPTY).inc();
    }
    match retired_count.min(3) {
        0 => state.stats.counter(paths::core::commit::RETIRE_HIST_ZERO).inc(),
        1 => state.stats.counter(paths::core::commit::RETIRE_HIST_ONE).inc(),
        2 => state.stats.counter(paths::core::commit::RETIRE_HIST_TWO).inc(),
        _ => state.stats.counter(paths::core::commit::RETIRE_HIST_THREE_PLUS).inc(),
    }

    // One drain per cycle: fall through to VSB if scalar SB has nothing committed.
    if !try_drain_one_store(state, common, store_buffer)
        && let Some(vsb) = vec_store_buffer
    {
        let _ = vsb.drain_one_committed(state, common);
    }
    trap_event
}

/// Drains one committed scalar SB entry to memory by emitting a `MemReq`
/// (op = Write). Returns true if a write was emitted (so the caller can
/// decide whether to also drain the vec-store buffer in the same cycle).
fn try_drain_one_store(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
) -> bool {
    let Some(store) = store_buffer.drain_one() else { return false };
    let StoreResolution::Committed { paddr, data } = store.resolution else {
        // Cancelled (failed SC) — slot was drained without a write.
        return true;
    };

    // Only pure RAM addresses go through the WCB coalesce path. HTIF and
    // other MMIO overlays must bypass it so the per-store MemReq carries the
    // original data to the device (WCB drain packets carry zero data).
    let width_bytes = width_to_bytes(store.width);
    let is_pure_ram = state.bus.ram_region_for(paddr.val(), width_bytes as u64).is_some();

    if !state.core.wcb.is_disabled() && is_pure_ram {
        // Update RAM directly so subsequent loads via the fast path see the
        // new value while the WCB coalesces dirty-line accounting; the WCB
        // drain only signals the line was dirty, it doesn't carry data.
        write_store_data_to_ram(state, paddr, data, store.width);
        let evicted = state.core.wcb.merge_store(paddr, data, width_bytes);
        if evicted.is_none() {
            state.stats.counter(paths::core::wcb::COALESCES).inc();
        }
        if let Some(drain) = evicted {
            emit_line_writeback(state, common, PhysAddr::new(drain.line_addr));
            state.stats.counter(paths::core::wcb::DRAINS).inc();
        }
    } else {
        write_store_to_memory(state, common, paddr, data, store.width);
    }
    trace_commit!(state.config.general.trace_instructions;
        paddr      = %crate::trace::Hex(paddr.val()),
        data       = %crate::trace::Hex(data),
        width      = ?store.width,
        via_wcb    = !state.core.wcb.is_disabled() && is_pure_ram,
        "CM: committed store drained to memory"
    );
    true
}

/// Drains **all** committed stores from the store buffer through the packet
/// pipeline, and flushes any committed vec-store writes from the VSB too.
/// Also flushes the WCB.
///
/// Called before SATP writes (so the PTW sees up-to-date PTEs) and on FENCE
/// commit (so younger memory ops see older committed writes).
fn drain_all_committed(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
    vec_store_buffer: Option<&mut crate::core::pipeline::vec_store_buffer::VecStoreBuffer>,
) {
    while let Some(store) = store_buffer.drain_one() {
        if let StoreResolution::Committed { paddr, data } = store.resolution {
            write_store_to_memory(state, common, paddr, data, store.width);
        }
    }
    if let Some(vsb) = vec_store_buffer {
        vsb.drain_all_committed(state, common);
    }
    flush_wcb(state, common);
}

/// Flushes all WCB entries by emitting write-back `MemReq` packets.
fn flush_wcb(state: &mut CoreCtx<'_>, common: &mut BackendCommon) {
    let drains = state.core.wcb.flush_all();
    for drain in drains {
        emit_line_writeback(state, common, PhysAddr::new(drain.line_addr));
        state.stats.counter(paths::core::wcb::DRAINS).inc();
    }
}

/// Emits a cache-line write-back `MemReq` to the L1D for an evicted WCB
/// line. The cache routes it through the hierarchy; memctrl applies the
/// actual DRAM write.
fn emit_line_writeback(state: &mut CoreCtx<'_>, common: &mut BackendCommon, paddr: PhysAddr) {
    let req_id = common.alloc_req_id();
    let l1_d_id = common.l1_d_id;
    let pipeline_id = common.pipeline_id;
    let _ = common.outstanding_stores.insert(
        req_id,
        OutstandingStore { rob_tag: crate::core::pipeline::rob::RobTag::default(), paddr },
    );
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        ComponentId::Cache(l1_d_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: AccessSize::Line,
            op: MemOp::Write { data: WriteData::Small(0) },
        },
    );
}

/// Writes a store's data to the correct memory target (RAM fast-path or bus).
/// Resolves and applies a CBO instruction at commit. Returns `Some(trap)` if
/// the instruction must trap; `None` if it completed successfully. Caller
/// must have drained the store buffer first so prior committed stores are
/// visible to the page-table walk and observable by other agents before
/// this op's side effect.
fn commit_cbo(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    op: SystemOp,
    rs1: u64,
    inst: u32,
) -> Option<Trap> {
    use crate::common::{AccessType, VirtAddr};
    use crate::core::arch::csr::{
        CboInvalAction, cbo_inval_action, cbocf_allowed, cboz_allowed,
    };
    use crate::isa::zicboz::CBOZ_BLOCK_SIZE;

    let (effective_op, access) = match op {
        SystemOp::CboZero => {
            if !cboz_allowed(state.hart.csrs.menvcfg, state.hart.csrs.senvcfg, state.hart.privilege) {
                return Some(Trap::IllegalInstruction(inst));
            }
            (SystemOp::CboZero, AccessType::Write)
        }
        SystemOp::CboInval => match cbo_inval_action(
            state.hart.csrs.menvcfg,
            state.hart.csrs.senvcfg,
            state.hart.privilege,
        ) {
            CboInvalAction::Illegal => return Some(Trap::IllegalInstruction(inst)),
            CboInvalAction::Flush => (SystemOp::CboFlush, AccessType::Read),
            CboInvalAction::Invalidate => (SystemOp::CboInval, AccessType::Write),
        },
        SystemOp::CboClean | SystemOp::CboFlush => {
            if !cbocf_allowed(state.hart.csrs.menvcfg, state.hart.csrs.senvcfg, state.hart.privilege) {
                return Some(Trap::IllegalInstruction(inst));
            }
            (op, AccessType::Read)
        }
        _ => return None,
    };

    let aligned_va = rs1 & !(CBOZ_BLOCK_SIZE - 1);
    let translate_result = state.translate(VirtAddr::new(aligned_va), access, CBOZ_BLOCK_SIZE);
    let result = match translate_result {
        crate::sim::state::memory::TranslateResult::Ready(r) => r,
        crate::sim::state::memory::TranslateResult::NeedPte { .. } => {
            // Commit-time walks are not yet pipelined for CBO; surface as
            // a page fault so the trap commits and the next attempt warms
            // the TLB via a regular load.
            return Some(match access {
                AccessType::Read => Trap::LoadPageFault(aligned_va),
                AccessType::Write => Trap::StorePageFault(aligned_va),
                AccessType::Fetch => Trap::InstructionPageFault(aligned_va),
            });
        }
    };
    if let Some(trap) = result.trap {
        return Some(trap);
    }
    let paddr = result.paddr.val();

    match effective_op {
        SystemOp::CboZero => cboz_write(state, common, paddr),
        // cbo.flush is "writeback then invalidate"; in this simulator stores
        // are already at RAM by commit, so the writeback is a no-op and we
        // share cbo.inval's drop-the-line implementation.
        SystemOp::CboInval | SystemOp::CboFlush => {
            let _ = state.core.l1_d_cache.invalidate_line(paddr);
        }
        SystemOp::CboClean => {
            let _ = state.core.l1_d_cache.clean_line(paddr);
        }
        _ => {}
    }
    None
}

/// Writes `CBOZ_BLOCK_SIZE` bytes of zeros at `block_paddr` as a sequence of
/// 8-byte stores. Caller must drain the store buffer first.
fn cboz_write(state: &mut CoreCtx<'_>, common: &mut BackendCommon, block_paddr: u64) {
    use crate::isa::zicboz::CBOZ_BLOCK_SIZE;
    const CHUNK: u64 = 8;
    let mut offset = 0u64;
    while offset < CBOZ_BLOCK_SIZE {
        write_store_to_memory(
            state,
            common,
            PhysAddr::new(block_paddr + offset),
            0,
            MemWidth::Double,
        );
        offset += CHUNK;
    }
}

/// Writes a store's data to memory by emitting a `MemReq` (op = Write).
///
/// Data dimension: for RAM-backed addresses the bytes are written directly
/// into the `RamRegion` here so subsequent loads via the RAM fast path see
/// the new value. MMIO addresses are not backed by RAM, so the device's
/// `Handle::handle` runs the side effect when the packet reaches it.
///
/// Latency / state dimension: a `Packet::MemReq` flows through the cache
/// hierarchy regardless so the L1D dirty bit, MSHR, write-combining buffer,
/// memory-controller accounting, and outstanding-store ack all see the
/// store at the right time.
fn write_store_to_memory(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
) {
    let access_size = match width {
        MemWidth::Byte => AccessSize::B1,
        MemWidth::Half => AccessSize::B2,
        MemWidth::Word => AccessSize::B4,
        MemWidth::Double => AccessSize::B8,
        MemWidth::Nop => return,
    };
    let width_bytes = match width {
        MemWidth::Byte => 1u64,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => return,
    };

    write_store_data_to_ram(state, paddr, data, width);

    let is_ram = state.bus.ram_region_for(paddr.val(), width_bytes).is_some();
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;
    let target = if is_ram {
        ComponentId::Cache(common.l1_d_id)
    } else {
        ComponentId::Bus
    };
    let _ = common.outstanding_stores.insert(
        req_id,
        OutstandingStore { rob_tag: crate::core::pipeline::rob::RobTag::default(), paddr },
    );
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: access_size,
            op: MemOp::Write { data: WriteData::Small(data) },
        },
    );
}

/// Writes a committed store's bytes into the `RamRegion` fast-path so subsequent
/// loads (which read RAM directly via `read_load_bytes`) see the new value.
/// No-op for addresses outside RAM (MMIO) — those reach their device via packet.
fn write_store_data_to_ram(state: &CoreCtx<'_>, paddr: PhysAddr, data: u64, width: MemWidth) {
    let width_bytes = match width {
        MemWidth::Byte => 1u64,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => return,
    };
    let Some(r) = state.bus.ram_region_for(paddr.val(), width_bytes) else { return };
    // SAFETY: `ram_region_for` confirms pure-RAM coverage and bounds-checks.
    unsafe {
        let ptr = r.ptr(paddr.val());
        match width {
            MemWidth::Byte => *ptr = data as u8,
            MemWidth::Half => ptr.cast::<u16>().write_unaligned(data as u16),
            MemWidth::Word => ptr.cast::<u32>().write_unaligned(data as u32),
            MemWidth::Double => ptr.cast::<u64>().write_unaligned(data),
            MemWidth::Nop => {}
        }
    }
}

/// Checks for pending interrupts. Returns the trap if one should be taken.
fn check_interrupts(state: &CoreCtx<'_>) -> Option<Trap> {
    let mip = state.hart.csrs.mip;
    let mie = state.hart.csrs.mie;
    let mstatus = state.hart.csrs.mstatus;

    let m_global_ie = (mstatus & csr::MSTATUS_MIE) != 0;
    let s_global_ie = (mstatus & csr::MSTATUS_SIE) != 0;

    let check = |bit: u64, enable_bit: u64, deleg_bit: u64| -> Option<Trap> {
        let pending = (mip & bit) != 0;
        let enabled = (mie & enable_bit) != 0;
        if !pending || !enabled {
            return None;
        }

        let delegated = (state.hart.csrs.mideleg & deleg_bit) != 0;
        let target_priv =
            if delegated { PrivilegeMode::Supervisor } else { PrivilegeMode::Machine };

        if state.hart.privilege.to_u8() < target_priv.to_u8() {
            return Some(TrapHandler::irq_to_trap(bit));
        }
        if state.hart.privilege == target_priv {
            if target_priv == PrivilegeMode::Machine && m_global_ie {
                return Some(TrapHandler::irq_to_trap(bit));
            }
            if target_priv == PrivilegeMode::Supervisor && s_global_ie {
                return Some(TrapHandler::irq_to_trap(bit));
            }
        }
        None
    };

    check(csr::MIP_MEIP, csr::MIE_MEIP, 1 << DELEG_MEIP_BIT)
        .or_else(|| check(csr::MIP_MSIP, csr::MIE_MSIP, 1 << DELEG_MSIP_BIT))
        .or_else(|| check(csr::MIP_MTIP, csr::MIE_MTIE, 1 << DELEG_MTIP_BIT))
        .or_else(|| check(csr::MIP_SEIP, csr::MIE_SEIP, 1 << DELEG_SEIP_BIT))
        .or_else(|| check(csr::MIP_SSIP, csr::MIE_SSIP, 1 << DELEG_SSIP_BIT))
        .or_else(|| check(csr::MIP_STIP, csr::MIE_STIE, 1 << DELEG_STIP_BIT))
}

/// Updates instruction statistics based on the committed entry.
fn update_instruction_stats(state: &mut CoreCtx<'_>, entry: &crate::core::pipeline::rob::RobEntry) {
    // Check vec ops first: vec loads/stores also set mem_read/mem_write.
    if !matches!(entry.ctrl.vec_op, VectorOp::None) {
        update_vec_instruction_stats(state, entry.ctrl.vec_op);
        return;
    }

    if entry.ctrl.mem_read {
        if entry.ctrl.fp_reg_write {
            state.stats.counter(paths::core::commit::FP_LOAD).inc();
        } else {
            state.stats.counter(paths::core::commit::OP_LOAD).inc();
        }
    } else if entry.ctrl.mem_write {
        if entry.ctrl.rs2_fp {
            state.stats.counter(paths::core::commit::FP_STORE).inc();
        } else {
            state.stats.counter(paths::core::commit::OP_STORE).inc();
        }
    } else if matches!(entry.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump) {
        state.stats.counter(paths::core::commit::OP_BRANCH).inc();
    } else if !matches!(entry.ctrl.system_op, SystemOp::None) {
        state.stats.counter(paths::core::commit::OP_SYSTEM).inc();
    } else {
        match entry.ctrl.alu {
            AluOp::FAdd
            | AluOp::FSub
            | AluOp::FMul
            | AluOp::FMin
            | AluOp::FMax
            | AluOp::FSgnJ
            | AluOp::FSgnJN
            | AluOp::FSgnJX
            | AluOp::FEq
            | AluOp::FLt
            | AluOp::FLe
            | AluOp::FClass
            | AluOp::FCvtWS
            | AluOp::FCvtWUS
            | AluOp::FCvtLS
            | AluOp::FCvtLUS
            | AluOp::FCvtSW
            | AluOp::FCvtSWU
            | AluOp::FCvtSL
            | AluOp::FCvtSLU
            | AluOp::FCvtSD
            | AluOp::FCvtDS
            | AluOp::FCvtSH
            | AluOp::FCvtHS
            | AluOp::FCvtDH
            | AluOp::FCvtHD
            | AluOp::FMvToX
            | AluOp::FMvToF => state.stats.counter(paths::core::commit::FP_ARITH).inc(),
            AluOp::FDiv | AluOp::FSqrt => {
                state.stats.counter(paths::core::commit::FP_DIV_SQRT).inc();
            }
            AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => {
                state.stats.counter(paths::core::commit::FP_FMA).inc();
            }
            _ => state.stats.counter(paths::core::commit::OP_ALU).inc(),
        }
    }
}

/// Categorize a vector instruction into the appropriate stat counter.
fn update_vec_instruction_stats(state: &mut CoreCtx<'_>, op: VectorOp) {
    match op {
        VectorOp::None => {}
        VectorOp::VLoadUnit
        | VectorOp::VLoadFF
        | VectorOp::VLoadMask
        | VectorOp::VLoadWholeReg
        | VectorOp::VLoadStride
        | VectorOp::VLoadIndexOrd
        | VectorOp::VLoadIndexUnord => {
            state.stats.counter(paths::core::commit::VEC_LOAD).inc();
        }
        VectorOp::VStoreUnit
        | VectorOp::VStoreMask
        | VectorOp::VStoreWholeReg
        | VectorOp::VStoreStride
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
            state.stats.counter(paths::core::commit::VEC_STORE).inc();
        }
        VectorOp::VAdd
        | VectorOp::VSub
        | VectorOp::VRsub
        | VectorOp::VAnd
        | VectorOp::VOr
        | VectorOp::VXor
        | VectorOp::VSll
        | VectorOp::VSrl
        | VectorOp::VSra
        | VectorOp::VMinU
        | VectorOp::VMin
        | VectorOp::VMaxU
        | VectorOp::VMax
        | VectorOp::VMerge
        | VectorOp::VMSeq
        | VectorOp::VMSne
        | VectorOp::VMSltu
        | VectorOp::VMSlt
        | VectorOp::VMSleu
        | VectorOp::VMSle
        | VectorOp::VMSgtu
        | VectorOp::VMSgt
        | VectorOp::VAdc
        | VectorOp::VMadc
        | VectorOp::VSbc
        | VectorOp::VMsbc
        | VectorOp::VSAddU
        | VectorOp::VSAdd
        | VectorOp::VSSubU
        | VectorOp::VSSub
        | VectorOp::VAAddU
        | VectorOp::VAAdd
        | VectorOp::VASubU
        | VectorOp::VASub
        | VectorOp::VSmul
        | VectorOp::VSSrl
        | VectorOp::VSSra
        | VectorOp::VZextVf2
        | VectorOp::VZextVf4
        | VectorOp::VZextVf8
        | VectorOp::VSextVf2
        | VectorOp::VSextVf4
        | VectorOp::VSextVf8
        | VectorOp::VNSrl
        | VectorOp::VNSra
        | VectorOp::VNClipU
        | VectorOp::VNClip
        | VectorOp::VMul
        | VectorOp::VMulh
        | VectorOp::VMulhu
        | VectorOp::VMulhsu
        | VectorOp::VMacc
        | VectorOp::VNMSac
        | VectorOp::VMadd
        | VectorOp::VNMSub
        | VectorOp::VDivU
        | VectorOp::VDiv
        | VectorOp::VRemU
        | VectorOp::VRem
        | VectorOp::VWAddU
        | VectorOp::VWAdd
        | VectorOp::VWSubU
        | VectorOp::VWSub
        | VectorOp::VWAddUW
        | VectorOp::VWAddW
        | VectorOp::VWSubUW
        | VectorOp::VWSubW
        | VectorOp::VWMulU
        | VectorOp::VWMul
        | VectorOp::VWMulSU
        | VectorOp::VWMaccU
        | VectorOp::VWMacc
        | VectorOp::VWMaccSU
        | VectorOp::VWMaccUS
        | VectorOp::VRedSum
        | VectorOp::VRedAnd
        | VectorOp::VRedOr
        | VectorOp::VRedXor
        | VectorOp::VRedMinU
        | VectorOp::VRedMin
        | VectorOp::VRedMaxU
        | VectorOp::VRedMax
        | VectorOp::VWRedSumU
        | VectorOp::VWRedSum => state.stats.counter(paths::core::commit::VEC_INT).inc(),
        VectorOp::VFAdd
        | VectorOp::VFSub
        | VectorOp::VFRSub
        | VectorOp::VFMul
        | VectorOp::VFDiv
        | VectorOp::VFRDiv
        | VectorOp::VFMin
        | VectorOp::VFMax
        | VectorOp::VFSgnj
        | VectorOp::VFSgnjn
        | VectorOp::VFSgnjx
        | VectorOp::VMFEq
        | VectorOp::VMFNe
        | VectorOp::VMFLt
        | VectorOp::VMFLe
        | VectorOp::VMFGt
        | VectorOp::VMFGe
        | VectorOp::VFSqrt
        | VectorOp::VFRsqrt7
        | VectorOp::VFRec7
        | VectorOp::VFClass
        | VectorOp::VFCvtXuF
        | VectorOp::VFCvtXF
        | VectorOp::VFCvtFXu
        | VectorOp::VFCvtFX
        | VectorOp::VFCvtRtzXuF
        | VectorOp::VFCvtRtzXF
        | VectorOp::VFMacc
        | VectorOp::VFNMacc
        | VectorOp::VFMSac
        | VectorOp::VFNMSac
        | VectorOp::VFMAdd
        | VectorOp::VFNMAdd
        | VectorOp::VFMSub
        | VectorOp::VFNMSub
        | VectorOp::VFWAdd
        | VectorOp::VFWSub
        | VectorOp::VFWMul
        | VectorOp::VFWAddW
        | VectorOp::VFWSubW
        | VectorOp::VFWMacc
        | VectorOp::VFWNMacc
        | VectorOp::VFWMSac
        | VectorOp::VFWNMSac
        | VectorOp::VFWCvtXuF
        | VectorOp::VFWCvtXF
        | VectorOp::VFWCvtFXu
        | VectorOp::VFWCvtFX
        | VectorOp::VFWCvtFF
        | VectorOp::VFWCvtRtzXuF
        | VectorOp::VFWCvtRtzXF
        | VectorOp::VFNCvtXuF
        | VectorOp::VFNCvtXF
        | VectorOp::VFNCvtFXu
        | VectorOp::VFNCvtFX
        | VectorOp::VFNCvtFF
        | VectorOp::VFNCvtRodFF
        | VectorOp::VFNCvtRtzXuF
        | VectorOp::VFNCvtRtzXF
        | VectorOp::VFMerge
        | VectorOp::VFMvSF
        | VectorOp::VFMvFS
        | VectorOp::VFSlide1Up
        | VectorOp::VFSlide1Down
        | VectorOp::VFRedOSum
        | VectorOp::VFRedUSum
        | VectorOp::VFRedMax
        | VectorOp::VFRedMin
        | VectorOp::VFWRedOSum
        | VectorOp::VFWRedUSum => state.stats.counter(paths::core::commit::VEC_FP).inc(),
        VectorOp::Vsetvli
        | VectorOp::Vsetivli
        | VectorOp::Vsetvl
        | VectorOp::VMAndMM
        | VectorOp::VMNandMM
        | VectorOp::VMAndnMM
        | VectorOp::VMOrMM
        | VectorOp::VMNorMM
        | VectorOp::VMOrnMM
        | VectorOp::VMXorMM
        | VectorOp::VMXnorMM
        | VectorOp::VCPopM
        | VectorOp::VFirstM
        | VectorOp::VMSbfM
        | VectorOp::VMSifM
        | VectorOp::VMSofM
        | VectorOp::VIotaM
        | VectorOp::VIdV
        | VectorOp::VMvXS
        | VectorOp::VMvSX
        | VectorOp::VSlideUp
        | VectorOp::VSlideDown
        | VectorOp::VSlide1Up
        | VectorOp::VSlide1Down
        | VectorOp::VRgather
        | VectorOp::VRgatherEi16
        | VectorOp::VCompress
        | VectorOp::VMv1r
        | VectorOp::VMv2r
        | VectorOp::VMv4r
        | VectorOp::VMv8r
        | VectorOp::VAndN
        | VectorOp::VBrev
        | VectorOp::VBrev8
        | VectorOp::VRev8
        | VectorOp::VClz
        | VectorOp::VCtz
        | VectorOp::VCpopV
        | VectorOp::VRol
        | VectorOp::VRor
        | VectorOp::VWsll
        | VectorOp::VClMul
        | VectorOp::VClMulH
        | VectorOp::VAesEm
        | VectorOp::VAesEf
        | VectorOp::VAesDm
        | VectorOp::VAesDf
        | VectorOp::VAesZ
        | VectorOp::VAesKf1
        | VectorOp::VAesKf2
        | VectorOp::VSha2Ms
        | VectorOp::VSha2Ch
        | VectorOp::VSha2Cl
        | VectorOp::VSm3Me
        | VectorOp::VSm3C
        | VectorOp::VSm4R
        | VectorOp::VSm4K
        | VectorOp::VGhsh
        | VectorOp::VGmul => state.stats.counter(paths::core::commit::VEC_MISC).inc(),
    }
}

/// Performs selective SFENCE.VMA TLB/cache flushing at commit time per the privileged spec:
/// rs1==0,rs2==0: flush all TLBs + D-cache + I-cache;
/// rs1!=0,rs2==0: flush TLB entries matching vaddr in rs1;
/// rs1==0,rs2!=0: flush non-global TLB entries matching ASID in rs2;
/// rs1!=0,rs2!=0: flush TLB entry matching both vaddr and ASID.
fn sfence_vma_commit(state: &mut CoreCtx<'_>, info: &SfenceVmaInfo) {
    match (!info.rs1_idx.is_zero(), !info.rs2_idx.is_zero()) {
        (false, false) => {
            state.hart.mmu.dtlb.flush();
            state.hart.mmu.itlb.flush();
            state.hart.mmu.l2_tlb.flush();
            let _ = state.core.l1_d_cache.flush();
            let _ = state.core.l1_i_cache.invalidate_all();
        }
        (true, false) => {
            let vpn = Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
            state.hart.mmu.dtlb.flush_vaddr(vpn);
            state.hart.mmu.itlb.flush_vaddr(vpn);
            state.hart.mmu.l2_tlb.flush_vaddr(vpn);
        }
        (false, true) => {
            let asid = Asid::new(info.rs2_val as u16);
            state.hart.mmu.dtlb.flush_asid(asid);
            state.hart.mmu.itlb.flush_asid(asid);
            state.hart.mmu.l2_tlb.flush_asid(asid);
        }
        (true, true) => {
            let vpn = Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
            let asid = Asid::new(info.rs2_val as u16);
            state.hart.mmu.dtlb.flush_vaddr_asid(vpn, asid);
            state.hart.mmu.itlb.flush_vaddr_asid(vpn, asid);
            state.hart.mmu.l2_tlb.flush_vaddr_asid(vpn, asid);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::InstSize;
    use crate::config::Config;
    use crate::sim::CoreCtx;

    #[test]
    fn test_check_interrupts_none() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let state = sys.core_ctx(0);

        assert!(check_interrupts(&state).is_none());
    }

    #[test]
    fn test_check_interrupts_m_mode() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.csrs.mip = csr::MIP_MEIP;
        state.hart.csrs.mie = csr::MIE_MEIP;
        state.hart.csrs.mstatus |= csr::MSTATUS_MIE;
        state.hart.privilege = PrivilegeMode::Machine;

        assert_eq!(check_interrupts(&state), Some(Trap::MachineExternalInterrupt));
    }

    #[test]
    fn test_check_interrupts_s_mode_delegated() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        state.hart.csrs.mip = csr::MIP_SEIP;
        state.hart.csrs.mie = csr::MIE_SEIP;
        state.hart.csrs.mstatus |= csr::MSTATUS_SIE;
        state.hart.csrs.mideleg |= 1 << DELEG_SEIP_BIT;
        state.hart.privilege = PrivilegeMode::Supervisor;

        assert_eq!(check_interrupts(&state), Some(Trap::SupervisorExternalInterrupt));
    }

    #[test]
    fn test_commit_stage_normal() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let mut rob = Rob::new(4);
        let mut store_buffer = StoreBuffer::new(4);
        let mut scoreboard = Scoreboard::new();
        let mut committed_rename_map = RenameMap::new();
        let mut free_list = FreeList::new(64, 32);

        let ctrl = crate::core::pipeline::signals::ControlSignals {
            reg_write: true,
            ..Default::default()
        };

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ctrl,
                crate::core::pipeline::prf::PhysReg(1),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();
        rob.complete(tag, 42);

        let mut redirect = false;
        let mut common = BackendCommon::default();
        let trap = commit_stage(
            &mut state,
            &mut common,
            &mut rob,
            &mut store_buffer,
            &mut scoreboard,
            &mut committed_rename_map,
            &mut free_list,
            1,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut redirect,
        );
        assert!(trap.is_none());
        assert_eq!(state.hart.regs.read(RegIdx::new(1)), 42);
    }
}

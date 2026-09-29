//! Commit Stage: retire instructions from ROB head.
//!
//! This stage retires the oldest instruction(s) from the ROB in program order:
//! 1. Write results to the register file.
//! 2. Apply deferred CSR writes.
//! 3. Mark store buffer entries as Committed.
//! 4. Handle traps/interrupts.
//! 5. Drain one committed store to memory per cycle.

use crate::arch::csr;
use crate::arch::regs::vpr::Vpr;
use crate::arch::reservation::LrScRecord;
use crate::arch::translation::PteUpdate;
use crate::common::{PhysAddr, crosses_cache_line};
use crate::exec::cbo::CboEffect;
use crate::exec::retire;
use crate::exec::signals::ControlFlow;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::op::{AluOp, MemWidth, SystemOp, VectorOp};
use crate::isa::privileged::Trap;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::VRegIdx;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, Maintenance, MemOp, Packet, WriteData, WriteOrigin};
use crate::system::CoreCtx;
use crate::system::debug::PC_TRACE_MAX;
use crate::trace_branch;
use crate::trace_commit;
use crate::trace_csr;
use crate::trace_trap;
use crate::uarch::pipeline::engine::{BackendCommon, PendingTrap, TrapProgress};
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::{StoreBuffer, StoreData, width_to_bytes};
use crate::uarch::pipeline::lsq::vec_store_buffer::{VSB_LINE_BYTES, VecStoreBuffer};
use crate::uarch::pipeline::lsq::write_buffer::{WcbLine, WriteCombiningBuffer};
use crate::uarch::pipeline::outstanding::{OutstandingStore, StoreOwner};
use crate::uarch::pipeline::rename::checkpoint::{CheckpointId, CheckpointTable};
use crate::uarch::pipeline::rename::free_list::FreeList;
use crate::uarch::pipeline::rename::map::RenameMap;
use crate::uarch::pipeline::rename::prf::{PhysReg, PhysRegFile};
use crate::uarch::pipeline::rename::scoreboard::Scoreboard;
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysRegFile;
use crate::uarch::pipeline::rob::{Rob, RobEntry, RobState, RobTag};

/// What stopped commit this cycle. The engine flushes and redirects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitEvent {
    /// Take `Trap` with the given EPC.
    Trap(Trap, u64),
    /// The LR or AMO at `pc` read a value another hart has since
    /// overwritten; squash it and everything younger and refetch from `pc`.
    ReExecute(u64),
    /// The instruction just retired changed state that every younger
    /// instruction was fetched or translated without (privilege, satp, the
    /// instruction memory, a reservation): squash everything younger and
    /// refetch from `pc`, whether or not the fetch PC already points there.
    SquashAfter(u64),
}

/// The backend state commit retires into.
#[derive(Debug)]
pub struct CommitResources<'a> {
    /// State both backends share.
    pub common: &'a mut BackendCommon,
    /// The reorder buffer retirement pops from.
    pub rob: &'a mut Rob,
    /// Scalar stores, marked committed at retire and drained to memory.
    pub store_buffer: &'a mut StoreBuffer,
    /// Vector stores, marked committed at retire and drained to memory.
    pub vec_store_buffer: &'a mut VecStoreBuffer,
    /// Instructions retired per cycle.
    pub width: usize,
    /// The destination tracking retirement releases.
    pub registers: CommitRegisters<'a>,
}

/// How the backend tracks the destination of every in-flight instruction,
/// which decides what retiring one releases.
#[derive(Debug)]
pub enum CommitRegisters<'a> {
    /// Results are already in the architectural file; retiring clears the
    /// tag that marked the register busy.
    Scoreboard(&'a mut Scoreboard),
    /// Results live in physical registers; retiring commits the mapping,
    /// frees the previous mapping and releases the per-instruction slots
    /// only a renaming backend allocates.
    Renamed {
        /// The committed architectural-to-physical mapping.
        rename_map: &'a mut RenameMap,
        /// Free scalar physical registers.
        free_list: &'a mut FreeList<PhysReg>,
        /// Scalar physical register values.
        prf: &'a mut PhysRegFile,
        /// In-flight loads, released as they retire.
        load_queue: &'a mut LoadQueue,
        /// Branch checkpoints, freed as their branch retires.
        checkpoints: &'a mut CheckpointTable,
        /// Vector physical register values.
        vec_prf: &'a mut VecPhysRegFile,
        /// Free vector physical registers.
        vec_free_list: &'a mut FreeList<VecPhysReg>,
    },
}

impl CommitRegisters<'_> {
    fn retire_scalar(&mut self, entry: &RobEntry, is_fp: bool) {
        match self {
            Self::Scoreboard(scoreboard) => scoreboard.clear_if_match(entry.rd, is_fp, entry.tag),
            Self::Renamed { rename_map, free_list, .. } => {
                if entry.old_phys_dst.0 != entry.phys_dst.0 {
                    free_list.reclaim(entry.old_phys_dst);
                }
                rename_map.set(entry.rd, is_fp, entry.phys_dst);
            }
        }
    }

    /// Retires the `i`th vector destination of `entry` into `vreg`.
    fn retire_vec(&mut self, vpr: &mut Vpr, entry: &RobEntry, i: usize, vreg: VRegIdx) {
        match self {
            Self::Scoreboard(scoreboard) => scoreboard.clear_vec_if_match(vreg, entry.tag),
            Self::Renamed { rename_map, vec_prf, vec_free_list, .. } => {
                vpr.write_bytes(vreg, vec_prf.read_bytes(entry.vec_phys_dst[i]));
                if entry.vec_old_phys_dst[i] != entry.vec_phys_dst[i] {
                    vec_free_list.reclaim(entry.vec_old_phys_dst[i]);
                }
                rename_map.set_vec(vreg, entry.vec_phys_dst[i]);
            }
        }
    }

    /// Frees the destinations of an entry that trapped instead of retiring.
    fn reclaim_faulted(&mut self, entry: &RobEntry) {
        let Self::Renamed { free_list, vec_free_list, .. } = self else {
            return;
        };
        if entry.phys_dst.0 != 0 {
            free_list.reclaim(entry.phys_dst);
        }
        for i in 0..entry.vec_dst_count as usize {
            if !entry.vec_phys_dst[i].is_zero() {
                vec_free_list.reclaim(entry.vec_phys_dst[i]);
            }
        }
    }

    fn release_load(&mut self, tag: RobTag) {
        if let Self::Renamed { load_queue, .. } = self {
            load_queue.deallocate(tag);
        }
    }

    fn free_checkpoint(&mut self, id: CheckpointId) {
        if let Self::Renamed { checkpoints, .. } = self {
            checkpoints.free(id);
        }
    }
}

/// Executes the Commit stage.
///
/// Retires up to `res.width` instructions from the ROB head per cycle.
/// Handles register writes, CSR application, trap dispatch, and
/// store buffer drain. Store drains emit `MemReq` packets through the
/// engine's `BackendCommon`.
pub fn commit_stage(state: &mut CoreCtx<'_>, res: CommitResources<'_>) -> Option<CommitEvent> {
    let CommitResources { common, rob, store_buffer, vec_store_buffer, width, mut registers } = res;
    let mut event: Option<CommitEvent> = None;
    let now = state.cycle;
    common.deliver_commit_notices(&mut state.core.branch_predictor, now);

    if let TrapProgress::Pending(pending) = &common.trap {
        state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
        if state.cycle < pending.taken_at {
            return None;
        }
        let pending = pending.clone();
        common.trap = TrapProgress::None;
        return take_pending_trap(state, pending);
    }

    // Always check, even with empty ROB (timer firing during a stall).
    {
        let epc = rob.peek_head().map_or(state.hart.pc, |head| head.pc);

        let interrupt =
            retire::pending_interrupt(state.hart).filter(|_| !device_access_in_flight(common, rob));
        if let Some(interrupt_trap) = interrupt {
            // Fetch stops and everything already fetched retires first
            // (gem5 waits for its instruction list to empty); a WFI's
            // successors are wrong-path and are not waited for.
            common.trap = TrapProgress::DrainingForInterrupt;
            let drained = state.hart.wfi_waiting || (rob.is_empty() && common.frontend_empty);
            if drained {
                trace_trap!(state.trace_trap_enabled(&interrupt_trap);
                    event      = "interrupt",
                    epc        = %crate::common::trace::Hex(epc),
                    cause      = ?interrupt_trap,
                    mip        = %crate::common::trace::Hex(state.hart.csrs.mip),
                    mie        = %crate::common::trace::Hex(state.hart.csrs.mie),
                    mstatus    = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                    priv_mode  = ?state.hart.privilege,
                    "CM: interrupt detected — pipeline drained"
                );
                state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
                return schedule_trap(state, common, interrupt_trap, epc);
            }
        } else if state.hart.wfi_waiting {
            common.trap = TrapProgress::None;
            // Commit stops while the WFI waits; what was fetched behind it is
            // refetched when it wakes.
            let pending = state.hart.csrs.mip;
            let enabled = state.hart.csrs.mie;
            state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
            if (pending & enabled) != 0 {
                state.hart.wfi_waiting = false;
                return Some(CommitEvent::SquashAfter(state.hart.pc));
            }
            state.uncore.stats.counter(state.core.stat_paths.pipeline.cycles_wfi).inc();
            return event;
        } else {
            common.trap = TrapProgress::None;
        }
    }

    if event.is_some() {
        state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
        return event;
    }

    let mut retired_count: usize = 0;
    let mut youngest_retired = None;
    let rob_empty_at_start = rob.peek_head().is_none();
    for _ in 0..width {
        let Some(head) = rob.peek_head() else { break };

        // A squash on its way will remove the head: it is wrong-path.
        if common.will_squash(head.tag) {
            break;
        }

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
                    use crate::isa::privileged::Trap;
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
                trace_trap!(state.trace_trap_enabled(the_trap);
                    event     = "sync-exception",
                    pc        = %crate::common::trace::Hex(entry.pc),
                    rob_tag   = entry.tag.0,
                    cause     = ?the_trap,
                    priv_mode = ?state.hart.privilege,
                    mstatus   = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                    "CM: synchronous exception at commit"
                );
                // Elements a vector load returned before the faulting one.
                if let Some(writes) = &entry.vec_writes {
                    writes.apply(state.hart.regs.vpr_mut());
                }
                if let Some(vstart) = entry.fault_vstart {
                    state.hart.csrs.vstart = vstart;
                }
                // Faulting entry was popped before the post-trap flush, so reclaim its phys_dst here.
                registers.reclaim_faulted(&entry);
                event = schedule_trap(state, common, the_trap.clone(), entry.pc);
            }
            break;
        }

        if head.state == RobState::Completed && lr_read_a_stale_line(state, head) {
            state.uncore.stats.counter(state.core.stat_paths.lsq.coherence_replays).inc();
            trace_trap!(state.config.general.trace_instructions;
                event   = "coherence-reexecute",
                pc      = %crate::common::trace::Hex(head.pc),
                rob_tag = head.tag.0,
                "CM: LR read a line another hart has since written — re-executing"
            );
            event = Some(CommitEvent::ReExecute(head.pc));
            break;
        }

        // Setting D must recheck the PTE the store was translated with.
        if head.state == RobState::Completed
            && head.dirty_updates.iter().any(|update| updated_pte(state, update).is_none())
        {
            trace_trap!(state.config.general.trace_instructions;
                event   = "pte-changed-reexecute",
                pc      = %crate::common::trace::Hex(head.pc),
                rob_tag = head.tag.0,
                "CM: store's PTE changed since its walk — re-executing"
            );
            event = Some(CommitEvent::ReExecute(head.pc));
            break;
        }

        // A barrier retires once every older store's write has completed.
        if waits_for_older_stores(head)
            && older_stores_pending(store_buffer, vec_store_buffer, &state.core.wcb)
        {
            break;
        }

        let Some(entry) = rob.commit_head() else { break };
        retired_count += 1;
        youngest_retired = Some(entry.seq);
        let val = entry.result.unwrap_or(0);

        // The architectural PC advances to the retired instruction's successor:
        // a taken branch's target, so an interrupt's EPC is right.
        state.hart.pc = match entry.ctrl.control_flow {
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
            pc         = %crate::common::trace::Hex(entry.pc),
            rd         = entry.rd.as_usize(),
            rd_phys    = entry.phys_dst.0,
            old_phys   = entry.old_phys_dst.0,
            result     = %crate::common::trace::Hex(entry.result.unwrap_or(0)),
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
                Some((entry.pc, entry.inst, has_rd, entry.rd.as_usize(), val))
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

        state.hart.csrs.count_retired();
        state.hart.instructions_retired += 1;
        let hart_paths = state.hart_paths();
        state.stats.counter(hart_paths.retired_insts).inc();
        update_instruction_stats(state, &entry);

        if entry.control_resolved {
            trace_branch!(state.config.general.trace_instructions;
                event         = "retire",
                pc            = %crate::common::trace::Hex(entry.pc),
                rob_tag       = entry.tag.0,
                actual_taken  = entry.bp_outcome.taken,
                actual_target = %crate::common::trace::Hex(entry.bp_target.unwrap_or(0)),
                mispredicted  = entry.bp_outcome.mispredicted,
                "CM: branch retired"
            );
            if entry.bp_outcome.mispredicted {
                state.uncore.stats.counter(state.core.stat_paths.bp.committed_mispredicts).inc();
            } else {
                state.uncore.stats.counter(state.core.stat_paths.bp.committed_hits).inc();
            }
        }

        debug_assert!(
            entry.result.is_some() || (!entry.ctrl.reg_write && !entry.ctrl.fp_reg_write),
            "CM: committing instruction with reg_write but no result: rob_tag={} pc={:#x}",
            entry.tag.0,
            entry.pc,
        );
        if entry.ctrl.fp_reg_write {
            retire::write_fp(state.hart, entry.rd, val);
            registers.retire_scalar(&entry, true);
            trace_commit!(state.config.general.trace_instructions;
                pc       = %crate::common::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                reg      = entry.rd.as_usize(),
                rd_phys  = entry.phys_dst.0,
                old_phys = entry.old_phys_dst.0,
                value    = %crate::common::trace::Hex(val),
                is_fp    = true,
                "CM: FP register write"
            );
        } else if entry.ctrl.reg_write && !entry.rd.is_zero() {
            retire::write_int(state.hart, entry.rd, val);
            registers.retire_scalar(&entry, false);
            trace_commit!(state.config.general.trace_instructions;
                pc       = %crate::common::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                reg      = entry.rd.as_usize(),
                rd_phys  = entry.phys_dst.0,
                old_phys = entry.old_phys_dst.0,
                value    = %crate::common::trace::Hex(val),
                is_fp    = false,
                "CM: integer register write"
            );
        }

        if let Some(writes) = &entry.vec_writes {
            retire::apply_vector_writes(state.hart, writes);
        }

        if entry.vec_dst_count > 0 {
            let vd_base = entry.ctrl.vd.as_u8();
            for i in 0..entry.vec_dst_count as usize {
                let vreg = VRegIdx::new(vd_base + i as u8);
                registers.retire_vec(state.hart.regs.vpr_mut(), &entry, i, vreg);
            }
            retire::mark_vector_retired(state.hart);
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

        for update in entry.dirty_updates.iter() {
            if let Some(pte) = updated_pte(state, update) {
                let _ = write_store_to_memory(
                    state,
                    common,
                    update.pte_addr,
                    pte,
                    MemWidth::Double,
                    StoreOwner::Untracked,
                );
            }
        }

        // Apply fp_flags before CSR writes to keep execute-time CSR reads of fflags consistent.
        retire::accrue_fp_flags(state.hart, entry.fp_flags);

        if entry.vxsat {
            state.hart.csrs.vxsat = 1;
        }

        if let Some(vl) = entry.vl_trim {
            state.hart.csrs.vl = vl;
        }

        if let Some(vector) = entry.vec_csr_update {
            retire::apply_vector_config(state.hart, vector);
        }

        if let Some(csr_update) = entry.csr_update {
            // O3 applies fflags/fcsr eagerly at complete time; don't re-apply.
            if !csr_update.applied {
                let pc_before = state.hart.pc;
                state.csr_write(csr_update.addr, csr_update.new_val);
                // A write that trapped (the simulator panic CSR) moved the PC to a handler.
                if state.hart.pc != pc_before {
                    event = Some(CommitEvent::SquashAfter(state.hart.pc));
                    break;
                }
            }
            trace_csr!(state.config.general.trace_instructions;
                op       = if csr_update.applied { "write-eager" } else { "write-deferred" },
                pc       = %crate::common::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                csr_addr = %crate::common::trace::Hex32(csr_update.addr.as_u32()),
                old_val  = %crate::common::trace::Hex(csr_update.old_val),
                new_val  = %crate::common::trace::Hex(csr_update.new_val),
                deferred = !csr_update.applied,
                "CM: CSR write applied at commit"
            );
            // SATP redirect: post-execute fetches used old tables; refetch from the next instruction.
            if csr_update.addr == csr::SATP {
                event =
                    Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            }
            break;
        }

        if entry.ctrl.system_op == SystemOp::Mret {
            state.do_mret();
            trace_trap!(state.config.general.trace_instructions;
                event      = "return",
                insn       = "MRET",
                pc         = %crate::common::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::common::trace::Hex(state.hart.pc),
                mstatus    = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: MRET committed — privilege restored"
            );
            event = Some(CommitEvent::SquashAfter(state.hart.pc));
            break;
        }
        if entry.ctrl.system_op == SystemOp::Sret {
            state.do_sret();
            trace_trap!(state.config.general.trace_instructions;
                event      = "return",
                insn       = "SRET",
                pc         = %crate::common::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::common::trace::Hex(state.hart.pc),
                mstatus    = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: SRET committed — privilege restored"
            );
            event = Some(CommitEvent::SquashAfter(state.hart.pc));
            break;
        }

        if entry.ctrl.system_op == SystemOp::Wfi {
            if !retire::wfi(state.hart) {
                event =
                    Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            }
            break;
        }

        // A reservation is set as its LR retires, so a squashed LR leaves none.
        if let Some(LrScRecord::Lr { paddr }) = entry.lr_sc {
            state.set_reservation(paddr);
        }

        if entry.ctrl.uses_store_buffer() {
            // A hart's own store (or CBO) to its reservation set fails its
            // SC, which the spec allows; other harts' reservations break
            // when the store is performed. An SC or AMO already took effect
            // in the cache and has left the store buffer.
            if let Some(paddr) = store_buffer.find_paddr(entry.tag)
                && state.check_reservation(paddr)
            {
                state.clear_reservation();
            }
            store_buffer.mark_committed(entry.tag);
        } else if crate::exec::compute::vector::mem::is_vec_store(entry.ctrl.vec_op) {
            // Vector store data lives in the dedicated VecStoreBuffer.
            store_buffer.mark_committed(entry.tag);
            vec_store_buffer.mark_committed(entry.tag);
        }

        if entry.ctrl.mem_read || crate::exec::compute::vector::mem::is_vec_load(entry.ctrl.vec_op)
        {
            registers.release_load(entry.tag);
        }

        if let Some(ckpt_id) = entry.checkpoint_id {
            registers.free_checkpoint(ckpt_id);
        }

        if entry.ctrl.system_op == SystemOp::FenceI {
            // Older stores have completed (stall above); refills see them.
            state.core.l1_i_cache.invalidate_all();
            // FENCE.I serializes: younger instructions were fetched before it.
            event = Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            break;
        }

        // SFENCE.VMA: SB is empty (stall above). Flush TLBs, clear reservation, full squash.
        if let Some(info) = entry.sfence_vma {
            state.core.mmu.sfence_vma(&info);
            state.clear_reservation();
            event = Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            break;
        }

        state.hart.regs.write(RegIdx::new(0), 0);
    }

    if let Some(seq) = youngest_retired {
        common.note_committed(seq, state.cycle);
    }
    if retired_count == 0 && rob_empty_at_start {
        state.uncore.stats.counter(state.core.stat_paths.pipeline.cycles_rob_empty).inc();
    }
    match retired_count.min(3) {
        0 => state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc(),
        1 => state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_one).inc(),
        2 => state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_two).inc(),
        _ => state.uncore.stats.counter(state.core.stat_paths.commit.retire_hist_three_plus).inc(),
    }

    send_one_write(state, common, store_buffer, vec_store_buffer);
    event
}

/// True when the LR at the ROB head read its line before another hart
/// wrote it, so the reservation it would set is already broken. Plain loads
/// are not checked: RVWMO lets them keep the earlier value.
fn lr_read_a_stale_line(state: &CoreCtx<'_>, head: &RobEntry) -> bool {
    let Some(log) = state.memory.write_log() else { return false };
    let (Some(observed), Some(LrScRecord::Lr { paddr })) = (head.observed, head.lr_sc) else {
        return false;
    };
    log.written_by_other_since(paddr, state.hart.hart_id, observed)
}

/// The PTE a hardware A/D update writes, or `None` when the PTE has changed
/// since the walk that produced it.
fn updated_pte(state: &CoreCtx<'_>, update: &PteUpdate) -> Option<u64> {
    read_ram_word(state, update.pte_addr, MemWidth::Double)
        .and_then(|current| update.applied_to(current))
}

/// The word at `paddr` as it is in RAM right now; `None` outside pure RAM.
fn read_ram_word(state: &CoreCtx<'_>, paddr: PhysAddr, width: MemWidth) -> Option<u64> {
    if width == MemWidth::Nop {
        return None;
    }
    let region = state.bus.ram_region_for(paddr.val(), width.bytes())?;
    // SAFETY: `ram_region_for` confirms pure-RAM coverage and bounds-checks.
    let raw = unsafe {
        let ptr = region.ptr(paddr.val());
        match width {
            MemWidth::Byte => u64::from(*ptr),
            MemWidth::Half => u64::from(ptr.cast::<u16>().read_unaligned()),
            MemWidth::Word => u64::from(ptr.cast::<u32>().read_unaligned()),
            MemWidth::Double => ptr.cast::<u64>().read_unaligned(),
            MemWidth::Nop => 0,
        }
    };
    Some(raw)
}

/// Takes `trap` now when there is no trap latency, else parks it for
/// commit to take once the latency has elapsed.
fn schedule_trap(
    state: &CoreCtx<'_>,
    common: &mut BackendCommon,
    trap: Trap,
    epc: u64,
) -> Option<CommitEvent> {
    let latency = state.config.pipeline.trap_latency;
    if latency == 0 {
        return Some(CommitEvent::Trap(trap, epc));
    }
    common.trap = TrapProgress::Pending(PendingTrap { trap, epc, taken_at: state.cycle + latency });
    None
}

/// Takes a parked trap. An interrupt is what is pending and enabled now,
/// which may differ from what was detected, or be nothing at all.
fn take_pending_trap(state: &mut CoreCtx<'_>, pending: PendingTrap) -> Option<CommitEvent> {
    let (is_interrupt, _) = pending.trap.cause();
    let trap = if is_interrupt { retire::pending_interrupt(state.hart)? } else { pending.trap };
    if is_interrupt {
        state.hart.wfi_waiting = false;
    }
    Some(CommitEvent::Trap(trap, pending.epc))
}

/// True when the ROB head has a device read outstanding. Such a read was
/// issued non-speculatively and has already had its side effect, so the
/// instruction must retire before an interrupt can pre-empt it.
fn device_access_in_flight(common: &BackendCommon, rob: &Rob) -> bool {
    rob.peek_head().is_some_and(|head| {
        common.outstanding_loads.values().any(|l| l.side_effecting && l.entry.rob_tag == head.tag)
    })
}

/// True when `[paddr, paddr + width)` is RAM with no MMIO overlay, i.e. a
/// write there can be published directly rather than through a device.
fn is_pure_ram(state: &CoreCtx<'_>, paddr: PhysAddr, width: MemWidth) -> bool {
    state.bus.ram_region_for(paddr.val(), width.bytes()).is_some()
}

/// Sends the write of the oldest committed store not yet sent. The entry
/// keeps its slot until the write is acknowledged. Returns true if a store
/// was taken (so the caller does not also drain the vec-store buffer this
/// cycle).
fn try_drain_one_store(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
) -> bool {
    let Some(write) = store_buffer.begin_write() else { return false };
    let requests = match write.data {
        StoreData::Bytes(data) => send_data_store(state, common, write.paddr, data, write.width),
        StoreData::Block(effect) => {
            // A CBO follows the older stores the WCB holds for its block.
            if state.core.wcb.request_send(write.paddr, CBOZ_BLOCK_SIZE as usize) {
                return false;
            }
            vec![send_block_op(state, common, write.paddr, effect)]
        }
    };
    store_buffer.issue_write(write, &requests);
    true
}

/// Sends a committed store's data: into the WCB for RAM when it is
/// enabled, else as its own writes. Returns the writes the store buffer
/// waits for.
fn send_data_store(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
) -> Vec<ReqId> {
    // MMIO, including the HTIF window over RAM, bypasses the WCB so its
    // device sees each store.
    let via_wcb = !state.core.wcb.is_disabled() && is_pure_ram(state, paddr, width);
    trace_commit!(state.config.general.trace_instructions;
        paddr      = %crate::common::trace::Hex(paddr.val()),
        data       = %crate::common::trace::Hex(data),
        width      = ?width,
        via_wcb    = via_wcb,
        "CM: committed store's write sent to memory"
    );
    if !via_wcb {
        let hart = WriteOrigin::Hart(state.hart.hart_id);
        let store = StoreWrite { origin: hart, owner: StoreOwner::StoreBuffer };
        return emit_store_write_packet(state, common, paddr, data, width, store);
    }
    let span = state.core.wcb.entry_bytes();
    for (part_paddr, part_data, part_bytes) in span_parts(paddr, data, width_to_bytes(width), span)
    {
        match state.core.wcb.merge_store(part_paddr, part_data, part_bytes) {
            Some(evicted) => send_wcb_line(state, common, &evicted),
            None => state.uncore.stats.counter(state.core.stat_paths.wcb.coalesces).inc(),
        }
    }
    Vec::new()
}

/// Sends a committed CBO to the L1D: `cbo.zero` as the hart's write of a
/// zeroed block, the others as a maintenance operation carried to memory.
fn send_block_op(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    block: PhysAddr,
    effect: CboEffect,
) -> ReqId {
    let maintain = |op| MemOp::Maintain { op, dirty: false };
    let op = match effect {
        CboEffect::Zero => MemOp::Write {
            data: WriteData::Line {
                bytes: vec![0; CBOZ_BLOCK_SIZE as usize].into(),
                mask: u64::MAX,
            },
            origin: WriteOrigin::Hart(state.hart.hart_id),
        },
        CboEffect::Clean => maintain(Maintenance::Clean),
        CboEffect::Flush => maintain(Maintenance::Flush),
        CboEffect::Invalidate => maintain(Maintenance::Invalidate),
    };
    let req_id = common.alloc_req_id();
    let _ = common
        .outstanding_stores
        .insert(req_id, OutstandingStore { owner: StoreOwner::StoreBuffer, paddr: block });
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        ComponentId::Cache(common.l1_d_id),
        ComponentId::Pipeline(common.pipeline_id),
        Packet::MemReq { req_id, paddr: block, vaddr: None, size: AccessSize::Line, op },
    );
    req_id
}

/// True when `head` must not retire before every older store's write has
/// completed: SFENCE.VMA (the walker must see earlier PTE stores), FENCE.I,
/// and a FENCE whose predecessor set includes writes. An atomic that needs
/// older stores written waits for them before it issues, and a CBO is
/// ordered with them in the store buffer.
fn waits_for_older_stores(head: &RobEntry) -> bool {
    matches!(head.ctrl.system_op, SystemOp::SfenceVma | SystemOp::FenceI)
        || (head.ctrl.system_op == SystemOp::Fence && fence_orders_stores(head.inst))
}

/// True when FENCE `inst`'s predecessor set includes writes (`pred.w`), so
/// it orders every older store before what follows it.
const fn fence_orders_stores(inst: u32) -> bool {
    (inst >> 24) & 0b0001 != 0
}

/// True while a committed scalar or vector store has not finished writing.
fn older_stores_pending(
    store_buffer: &StoreBuffer,
    vec_store_buffer: &VecStoreBuffer,
    wcb: &WriteCombiningBuffer,
) -> bool {
    store_buffer.has_committed_stores()
        || vec_store_buffer.has_committed_stores()
        || wcb.has_pending()
}

/// Sends this cycle's write to the L1D: a line the WCB must send now, else
/// the oldest committed scalar store, else a vector store's line, else, the
/// port being idle, the WCB's oldest line.
pub(crate) fn send_one_write(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
    vec_store_buffer: &mut VecStoreBuffer,
) {
    if !send_urgent_wcb_line(state, common)
        && !try_drain_one_store(state, common, store_buffer)
        && !drain_vec_store_line(state, common, vec_store_buffer)
    {
        send_oldest_wcb_line(state, common);
    }
}

/// True while a committed store has yet to finish writing.
pub(crate) fn committed_writes_pending(
    state: &CoreCtx<'_>,
    store_buffer: &StoreBuffer,
    vec_store_buffer: &VecStoreBuffer,
) -> bool {
    older_stores_pending(store_buffer, vec_store_buffer, &state.core.wcb)
}

/// Sends a line the WCB must write now. Returns whether one went.
fn send_urgent_wcb_line(state: &mut CoreCtx<'_>, common: &mut BackendCommon) -> bool {
    let Some(line) = state.core.wcb.take_urgent() else { return false };
    send_wcb_line(state, common, &line);
    true
}

/// Sends the WCB's least recently merged line, if it holds one.
fn send_oldest_wcb_line(state: &mut CoreCtx<'_>, common: &mut BackendCommon) {
    if let Some(line) = state.core.wcb.take_oldest() {
        send_wcb_line(state, common, &line);
    }
}

/// Writes a WCB line to the L1D as the hart's store, taking effect where
/// the cache serves it.
fn send_wcb_line(state: &mut CoreCtx<'_>, common: &mut BackendCommon, line: &WcbLine) {
    let span = state.core.wcb.entry_bytes();
    let paddr = PhysAddr::new(line.line_addr);
    let req_id = emit_line_write(
        state,
        common,
        paddr,
        &line.data[..span],
        line.mask,
        StoreOwner::WriteCombining,
    );
    state.core.wcb.sent(req_id, line.clone());
    state.uncore.stats.counter(state.core.stat_paths.wcb.drains).inc();
}

/// Writes the next line of a committed vector store: to RAM as one masked
/// line write, and to a device as the naturally aligned writes it must see.
/// Returns whether a line went.
fn drain_vec_store_line(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    vec_store_buffer: &mut VecStoreBuffer,
) -> bool {
    let Some((rob_tag, line)) = vec_store_buffer.take_drainable_line() else { return false };
    let paddr = PhysAddr::new(line.line_addr);
    let owner = StoreOwner::VecStoreBuffer;
    let requests = if state.bus.ram_region_for(paddr.val(), VSB_LINE_BYTES as u64).is_some() {
        vec![emit_line_write(state, common, paddr, &line.data, line.valid_mask, owner)]
    } else {
        let write = StoreWrite { origin: WriteOrigin::Hart(state.hart.hart_id), owner };
        line.natural_writes()
            .into_iter()
            .flat_map(|(paddr, data, width)| {
                emit_store_write_packet(state, common, paddr, data, width, write)
            })
            .collect()
    };
    vec_store_buffer.line_sent(rob_tag, requests);
    true
}

/// Sends the hart's write of the bytes of the line at `line` that `mask`
/// selects to the L1D. Returns the request.
fn emit_line_write(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    line: PhysAddr,
    bytes: &[u8],
    mask: u64,
    owner: StoreOwner,
) -> ReqId {
    let req_id = common.alloc_req_id();
    let _ = common.outstanding_stores.insert(req_id, OutstandingStore { owner, paddr: line });
    let origin = WriteOrigin::Hart(state.hart.hart_id);
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        ComponentId::Cache(common.l1_d_id),
        ComponentId::Pipeline(common.pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: line,
            vaddr: None,
            size: AccessSize::Line,
            op: MemOp::Write { data: WriteData::Line { bytes: bytes.into(), mask }, origin },
        },
    );
    req_id
}

/// Writes a store's bytes to RAM at once and emits its `MemReq`s for their
/// timing only: for writes the hardware makes at a single instant (a
/// page-table D-bit update, `cbo.zero`) and for a checkpoint drain, which
/// does not wait for the memory system. An MMIO address is left to the
/// device the packet reaches.
fn write_store_to_memory(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
    owner: StoreOwner,
) -> Vec<ReqId> {
    if width == MemWidth::Nop {
        return Vec::new();
    }
    state.publish_write(paddr, data, width);
    emit_store_write_packet(state, common, paddr, data, width, StoreWrite::placed(owner))
}

/// Emits the `MemReq`s (op = Write) carrying a store, one per cache line a
/// RAM store touches. Returns the requests.
fn emit_store_write_packet(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
    write: StoreWrite,
) -> Vec<ReqId> {
    if width == MemWidth::Nop {
        return Vec::new();
    }
    let width_bytes = width.bytes() as usize;
    if !is_pure_ram(state, paddr, width) {
        let target = ComponentId::Bus;
        return vec![emit_store_write_packet_to(
            state,
            common,
            paddr,
            data,
            width_bytes,
            write,
            target,
        )];
    }
    let l1d = ComponentId::Cache(common.l1_d_id);
    line_parts(state, paddr, data, width_bytes)
        .into_iter()
        .map(|(part_paddr, part_data, part_bytes)| {
            emit_store_write_packet_to(state, common, part_paddr, part_data, part_bytes, write, l1d)
        })
        .collect()
}

/// Whose write a store's requests carry and the buffer their
/// acknowledgements go back to.
#[derive(Clone, Copy)]
struct StoreWrite {
    origin: WriteOrigin,
    owner: StoreOwner,
}

impl StoreWrite {
    /// A write whose bytes are already in RAM.
    const fn placed(owner: StoreOwner) -> Self {
        Self { origin: WriteOrigin::Placed, owner }
    }
}

/// The byte ranges of a store's data that fall in each cache line it
/// touches: `(address, data shifted to start at that address, bytes)`.
fn line_parts(
    state: &CoreCtx<'_>,
    paddr: PhysAddr,
    data: u64,
    width_bytes: usize,
) -> Vec<(PhysAddr, u64, usize)> {
    span_parts(paddr, data, width_bytes, state.core.l1_d_cache.line_bytes())
}

/// The byte ranges of a store's data that fall in each aligned `span`-byte
/// block it touches: `(address, data shifted to start there, bytes)`.
fn span_parts(
    paddr: PhysAddr,
    data: u64,
    width_bytes: usize,
    span: usize,
) -> Vec<(PhysAddr, u64, usize)> {
    let span = span as u64;
    if !crosses_cache_line(paddr.val(), width_bytes as u64, span) {
        return vec![(paddr, data, width_bytes)];
    }
    let second = (paddr.val() | (span - 1)) + 1;
    let first_bytes = (second - paddr.val()) as usize;
    vec![
        (paddr, data, first_bytes),
        (PhysAddr::new(second), data >> (8 * first_bytes), width_bytes - first_bytes),
    ]
}

/// Emits one `MemReq` (op = Write) of `bytes` bytes to `target` and returns
/// its request.
fn emit_store_write_packet_to(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    bytes: usize,
    write: StoreWrite,
    target: ComponentId,
) -> ReqId {
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;
    let _ =
        common.outstanding_stores.insert(req_id, OutstandingStore { owner: write.owner, paddr });
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: AccessSize::of_bytes(bytes),
            op: MemOp::Write { data: WriteData::Small(data), origin: write.origin },
        },
    );
    req_id
}

/// Updates instruction statistics based on the committed entry.
fn update_instruction_stats(
    state: &mut CoreCtx<'_>,
    entry: &crate::uarch::pipeline::rob::RobEntry,
) {
    // Check vec ops first: vec loads/stores also set mem_read/mem_write.
    if !matches!(entry.ctrl.vec_op, VectorOp::None) {
        update_vec_instruction_stats(state, entry.ctrl.vec_op);
        return;
    }

    if entry.ctrl.mem_read {
        if entry.ctrl.fp_reg_write {
            state.uncore.stats.counter(state.core.stat_paths.commit.fp_load).inc();
        } else {
            state.uncore.stats.counter(state.core.stat_paths.commit.op_load).inc();
        }
    } else if entry.ctrl.mem_write {
        if entry.ctrl.rs2_fp {
            state.uncore.stats.counter(state.core.stat_paths.commit.fp_store).inc();
        } else {
            state.uncore.stats.counter(state.core.stat_paths.commit.op_store).inc();
        }
    } else if matches!(entry.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump) {
        state.uncore.stats.counter(state.core.stat_paths.commit.op_branch).inc();
    } else if !matches!(entry.ctrl.system_op, SystemOp::None) {
        state.uncore.stats.counter(state.core.stat_paths.commit.op_system).inc();
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
            | AluOp::FMvToF => {
                state.uncore.stats.counter(state.core.stat_paths.commit.fp_arith).inc();
            }
            AluOp::FDiv | AluOp::FSqrt => {
                state.uncore.stats.counter(state.core.stat_paths.commit.fp_div_sqrt).inc();
            }
            AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => {
                state.uncore.stats.counter(state.core.stat_paths.commit.fp_fma).inc();
            }
            _ => state.uncore.stats.counter(state.core.stat_paths.commit.op_alu).inc(),
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
            state.uncore.stats.counter(state.core.stat_paths.commit.vec_load).inc();
        }
        VectorOp::VStoreUnit
        | VectorOp::VStoreMask
        | VectorOp::VStoreWholeReg
        | VectorOp::VStoreStride
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
            state.uncore.stats.counter(state.core.stat_paths.commit.vec_store).inc();
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
        | VectorOp::VWRedSum => {
            state.uncore.stats.counter(state.core.stat_paths.commit.vec_int).inc();
        }
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
        | VectorOp::VFWRedUSum => {
            state.uncore.stats.counter(state.core.stat_paths.commit.vec_fp).inc();
        }
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
        | VectorOp::VGmul => {
            state.uncore.stats.counter(state.core.stat_paths.commit.vec_misc).inc();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::isa::instruction::InstSize;

    #[test]
    fn test_commit_stage_normal() {
        let config = Config::default();
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let mut rob = Rob::new(4);
        let mut store_buffer = StoreBuffer::new(4);
        let mut vec_store_buffer = VecStoreBuffer::new(
            4,
            crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreForwarding::Off,
        );
        let mut scoreboard = Scoreboard::new();

        let ctrl = crate::exec::signals::ControlSignals { reg_write: true, ..Default::default() };

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ctrl,
                crate::uarch::pipeline::rename::prf::PhysReg(1),
                crate::uarch::pipeline::rename::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();
        rob.complete(tag, 42);

        let mut common = BackendCommon::default();
        let trap = commit_stage(
            &mut state,
            CommitResources {
                common: &mut common,
                rob: &mut rob,
                store_buffer: &mut store_buffer,
                vec_store_buffer: &mut vec_store_buffer,
                width: 1,
                registers: CommitRegisters::Scoreboard(&mut scoreboard),
            },
        );
        assert!(trap.is_none());
        assert_eq!(state.hart.regs.read(RegIdx::new(1)), 42);
    }
}

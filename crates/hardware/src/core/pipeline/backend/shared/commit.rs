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
use crate::common::{Asid, LrScRecord, PhysAddr, PteUpdate, RegIdx, SfenceVmaInfo, Trap, Vpn};
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::arch::trap::TrapHandler;
use crate::core::arch::vpr::Vpr;
use crate::core::pipeline::backend::shared::cbo::{self, CboEffect};
use crate::core::pipeline::backend::shared::memory2;
use crate::core::pipeline::checkpoint::{CheckpointId, CheckpointTable};
use crate::core::pipeline::engine::{BackendCommon, PendingTrap, TrapProgress};
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::outstanding::OutstandingStore;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::{Rob, RobEntry, RobState, RobTag};
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::signals::{AluOp, AtomicOp, ControlFlow, MemWidth, SystemOp, VectorOp};
use crate::core::pipeline::store_buffer::{StoreBuffer, StoreResolution, width_to_bytes};
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::pipeline::vec_store_buffer::VecStoreBuffer;
use crate::core::units::cache::DirtyLine;
use crate::core::units::lsu::unaligned;
use crate::core::units::vpu::types::{VRegIdx, VecPhysReg};
use crate::sim::CoreCtx;
use crate::sim::components::ComponentId;
use crate::sim::packet::{AccessSize, MemOp, Packet, WriteData};
use crate::sim::per_hart_debug::PC_TRACE_MAX;
use crate::trace_branch;
use crate::trace_commit;
use crate::trace_csr;
use crate::trace_trap;

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

    /// Records a failed SC's result of 1 where a post-squash rename reads it.
    fn record_sc_failure(&mut self, phys_dst: PhysReg) {
        if let Self::Renamed { prf, .. } = self {
            prf.write(phys_dst, 1);
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
        state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
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

        let interrupt = check_interrupts(state).filter(|_| !device_access_in_flight(common, rob));
        if let Some(interrupt_trap) = interrupt {
            // Fetch stops and everything already fetched retires first
            // (gem5 waits for its instruction list to empty); a WFI's
            // successors are wrong-path and are not waited for.
            common.trap = TrapProgress::DrainingForInterrupt;
            let drained = state.hart.wfi_waiting || (rob.is_empty() && common.frontend_empty);
            if drained {
                trace_trap!(state.trace_trap_enabled(&interrupt_trap);
                    event      = "interrupt",
                    epc        = %crate::trace::Hex(epc),
                    cause      = ?interrupt_trap,
                    mip        = %crate::trace::Hex(state.hart.csrs.mip),
                    mie        = %crate::trace::Hex(state.hart.csrs.mie),
                    mstatus    = %crate::trace::Hex(state.hart.csrs.mstatus),
                    priv_mode  = ?state.hart.privilege,
                    "CM: interrupt detected — pipeline drained"
                );
                state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
                return schedule_trap(state, common, interrupt_trap, epc);
            }
        } else if state.hart.wfi_waiting {
            common.trap = TrapProgress::None;
            // Commit stops while the WFI waits; what was fetched behind it is
            // refetched when it wakes.
            let pending = state.hart.csrs.mip;
            let enabled = state.hart.csrs.mie;
            state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
            if (pending & enabled) != 0 {
                state.hart.wfi_waiting = false;
                return Some(CommitEvent::SquashAfter(state.hart.pc));
            }
            state.shared.stats.counter(state.core.stat_paths.pipeline.cycles_wfi).inc();
            return event;
        } else {
            common.trap = TrapProgress::None;
        }
    }

    if event.is_some() {
        state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc();
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
                trace_trap!(state.trace_trap_enabled(the_trap);
                    event     = "sync-exception",
                    pc        = %crate::trace::Hex(entry.pc),
                    rob_tag   = entry.tag.0,
                    cause     = ?the_trap,
                    priv_mode = ?state.hart.privilege,
                    mstatus   = %crate::trace::Hex(state.hart.csrs.mstatus),
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

        if head.state == RobState::Completed && observed_value_is_stale(state, head, store_buffer) {
            state.shared.stats.counter(state.core.stat_paths.lsq.coherence_replays).inc();
            trace_trap!(state.config.general.trace_instructions;
                event   = "coherence-reexecute",
                pc      = %crate::trace::Hex(head.pc),
                rob_tag = head.tag.0,
                "CM: LR/AMO read a value another hart has overwritten — re-executing"
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
                pc      = %crate::trace::Hex(head.pc),
                rob_tag = head.tag.0,
                "CM: store's PTE changed since its walk — re-executing"
            );
            event = Some(CommitEvent::ReExecute(head.pc));
            break;
        }

        // SFENCE.VMA must wait for committed stores to reach RAM so PTW sees current PTEs.
        if head.ctrl.system_op == SystemOp::SfenceVma && store_buffer.has_committed_stores() {
            break;
        }
        // CBO ops (Zicboz / Zicbom) drain prior committed stores first so
        // memory ordering against earlier writes matches a normal store and
        // any stale PTE in the SB has settled before we re-translate.
        if head.ctrl.system_op.is_cbo() && store_buffer.has_committed_stores() {
            break;
        }
        // An `rl` atomic's write is published at commit; older stores go first.
        if head.ctrl.release
            && (store_buffer.has_committed_stores() || vec_store_buffer.has_committed_stores())
        {
            break;
        }

        let Some(entry) = rob.commit_head() else { break };
        retired_count += 1;
        youngest_retired = Some(entry.seq);

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

        state.hart.csrs.count_retired();
        state.hart.instructions_retired += 1;
        let hart_paths = state.hart_paths();
        state.stats.counter(hart_paths.retired_insts).inc();
        update_instruction_stats(state, &entry);

        if entry.bp_update {
            trace_branch!(state.config.general.trace_instructions;
                event         = "retire",
                pc            = %crate::trace::Hex(entry.pc),
                rob_tag       = entry.tag.0,
                actual_taken  = entry.bp_outcome.taken,
                actual_target = %crate::trace::Hex(entry.bp_target.unwrap_or(0)),
                mispredicted  = entry.bp_outcome.mispredicted,
                "CM: branch retired"
            );
            if entry.bp_outcome.mispredicted {
                state.shared.stats.counter(state.core.stat_paths.bp.committed_mispredicts).inc();
            } else {
                state.shared.stats.counter(state.core.stat_paths.bp.committed_hits).inc();
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
            registers.retire_scalar(&entry, true);
            state.hart.csrs.mstatus =
                (state.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
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
            registers.retire_scalar(&entry, false);
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

        if let Some(writes) = &entry.vec_writes {
            writes.apply(state.hart.regs.vpr_mut());
            state.hart.csrs.mstatus =
                (state.hart.csrs.mstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
            state.hart.csrs.vstart = 0;
        }

        if entry.vec_dst_count > 0 {
            let vd_base = entry.ctrl.vd.as_u8();
            for i in 0..entry.vec_dst_count as usize {
                let vreg = VRegIdx::new(vd_base + i as u8);
                registers.retire_vec(state.hart.regs.vpr_mut(), &entry, i, vreg);
            }
            state.hart.csrs.mstatus =
                (state.hart.csrs.mstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
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

        for update in entry.dirty_updates.iter() {
            if let Some(pte) = updated_pte(state, update) {
                write_store_to_memory(state, common, update.pte_addr, pte, MemWidth::Double);
            }
        }

        // Apply fp_flags before CSR writes to keep execute-time CSR reads of fflags consistent.
        if entry.fp_flags != 0 {
            state.hart.csrs.fflags |= entry.fp_flags as u64;
            state.hart.csrs.mstatus =
                (state.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
        }

        if entry.vxsat {
            state.hart.csrs.vxsat = 1;
        }

        if let Some(vl) = entry.vl_trim {
            state.hart.csrs.vl = vl;
        }

        if let Some(vector) = entry.vec_csr_update {
            state.hart.csrs.vtype = vector.vtype;
            state.hart.csrs.vl = vector.vl;
            state.hart.csrs.vstart = 0;
            state.hart.csrs.mstatus =
                (state.hart.csrs.mstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
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
                pc       = %crate::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                csr_addr = %crate::trace::Hex32(csr_update.addr.as_u32()),
                old_val  = %crate::trace::Hex(csr_update.old_val),
                new_val  = %crate::trace::Hex(csr_update.new_val),
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
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::trace::Hex(state.hart.pc),
                mstatus    = %crate::trace::Hex(state.hart.csrs.mstatus),
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
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::trace::Hex(state.hart.pc),
                mstatus    = %crate::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: SRET committed — privilege restored"
            );
            event = Some(CommitEvent::SquashAfter(state.hart.pc));
            break;
        }

        if entry.ctrl.system_op == SystemOp::Wfi {
            if state.hart.csrs.mie != 0 || state.hart.csrs.mip != 0 {
                state.hart.wfi_waiting = true;
            } else {
                // Nothing enabled or pending — treat as NOP to avoid OpenSBI early-boot deadlock.
                event =
                    Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            }
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
                            registers.record_sc_failure(entry.phys_dst);
                        }
                        event = Some(CommitEvent::SquashAfter(
                            entry.pc.wrapping_add(entry.inst_size.as_u64()),
                        ));
                        break;
                    }
                }
            }
        }

        if entry.ctrl.mem_write {
            if let Some(paddr) = store_buffer.find_paddr(entry.tag) {
                // RISC-V §8.2: a non-LR/SC store to the reservation set must
                // fail any paired SC. Other harts' reservations break when
                // the store is published (at drain for plain stores, here
                // for SC and AMO).
                if entry.lr_sc.is_none() && state.check_reservation(paddr) {
                    state.clear_reservation();
                }
                if entry.ctrl.atomic_op != AtomicOp::None
                    && is_pure_ram(state, paddr, entry.ctrl.width)
                    && let Some(applied) = store_buffer.commit_applied(entry.tag)
                {
                    state.publish_write(applied.paddr, applied.data, applied.width);
                }
            }
            store_buffer.mark_committed(entry.tag);
        } else if crate::core::units::vpu::mem::is_vec_store(entry.ctrl.vec_op) {
            // Vector store data lives in the dedicated VecStoreBuffer.
            store_buffer.mark_committed(entry.tag);
            vec_store_buffer.mark_committed(entry.tag);
        }

        if entry.ctrl.mem_read || crate::core::units::vpu::mem::is_vec_load(entry.ctrl.vec_op) {
            registers.release_load(entry.tag);
        }

        if let Some(ckpt_id) = entry.checkpoint_id {
            registers.free_checkpoint(ckpt_id);
        }

        if entry.ctrl.system_op == SystemOp::FenceI {
            drain_all_committed(state, common, store_buffer, vec_store_buffer);
            // I-cache flush after drain so refills see new data; force a fresh redirect.
            let _ = state.core.l1_i_cache.invalidate_all();
            // FENCE.I serializes: younger instructions were fetched before the drain.
            event = Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            break;
        } else if entry.ctrl.system_op == SystemOp::Fence {
            let pred_bits = ((entry.inst >> 24) & 0xF) as u8;
            let pred_w = pred_bits & 0b0001 != 0;
            let pred_r = pred_bits & 0b0010 != 0;
            // pred.w drains the SB; pred.r is satisfied by commit order.
            if pred_w || pred_r {
                drain_all_committed(state, common, store_buffer, vec_store_buffer);
            }
        }

        // SFENCE.VMA: SB is empty (stall above). Flush TLBs, clear reservation, full squash.
        if let Some(info) = entry.sfence_vma {
            sfence_vma_commit(state, &info);
            state.clear_reservation();
            event = Some(CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
            break;
        }

        // A CBO (Zicboz / Zicbom) runs on the block memory1 translated, its
        // result, once the store buffer has drained (stall above).
        if entry.ctrl.system_op.is_cbo() {
            let block = PhysAddr::new(entry.result.unwrap_or(0));
            if let Some(trap) = commit_cbo(state, common, entry.ctrl.system_op, block, entry.inst) {
                state.trap(&trap, entry.pc);
                event = Some(CommitEvent::SquashAfter(state.hart.pc));
                break;
            }
        }

        state.hart.regs.write(RegIdx::new(0), 0);
    }

    if let Some(seq) = youngest_retired {
        common.note_committed(seq, state.cycle);
    }
    if retired_count == 0 && rob_empty_at_start {
        state.shared.stats.counter(state.core.stat_paths.pipeline.cycles_rob_empty).inc();
    }
    match retired_count.min(3) {
        0 => state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_zero).inc(),
        1 => state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_one).inc(),
        2 => state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_two).inc(),
        _ => state.shared.stats.counter(state.core.stat_paths.commit.retire_hist_three_plus).inc(),
    }

    // One drain per cycle: fall through to VSB if scalar SB has nothing committed.
    if !try_drain_one_store(state, common, store_buffer) {
        let _ = vec_store_buffer.drain_one_committed(state, common);
    }
    event
}

/// True when the LR or AMO at the ROB head took its value before another
/// hart wrote its line, so what it would commit is stale.
///
/// An LR is stale on any such write: the reservation it would set covers
/// the whole line. An AMO is stale only if the word it read has changed as
/// well: a real core holds the line for its read-modify-write, so a write
/// elsewhere in the line (or one that restored the same value) does not
/// perturb the result, and treating it as stale would let harts hammering
/// one lock word replay each other forever. Plain loads are not checked:
/// RVWMO lets them keep the earlier value.
fn observed_value_is_stale(
    state: &CoreCtx<'_>,
    head: &RobEntry,
    store_buffer: &StoreBuffer,
) -> bool {
    let Some(log) = state.write_log.as_ref() else { return false };
    let Some(observed) = head.observed else { return false };
    let reader = state.hart.hart_id;
    match (head.lr_sc, head.ctrl.atomic_op) {
        (Some(LrScRecord::Lr { paddr }), _) => log.written_by_other_since(paddr, reader, observed),
        (_, AtomicOp::None | AtomicOp::Sc) => false,
        (_, _) => {
            let Some(paddr) = store_buffer.find_paddr(head.tag) else { return false };
            if !log.written_by_other_since(paddr, reader, observed) {
                return false;
            }
            let Some(current) = read_ram_word(state, paddr, head.ctrl.width) else { return false };
            let current = memory2::sign_extend(current, head.ctrl.width, head.ctrl.signed_load);
            head.result != Some(current)
        }
    }
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
    let trap = if is_interrupt { check_interrupts(state)? } else { pending.trap };
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

/// Drains one committed scalar SB entry to memory by emitting a `MemReq`
/// (op = Write). Returns true if a write was emitted (so the caller can
/// decide whether to also drain the vec-store buffer in the same cycle).
fn try_drain_one_store(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
) -> bool {
    let Some(store) = store_buffer.drain_one() else { return false };
    let (paddr, data) = match store.resolution {
        StoreResolution::Committed { paddr, data } => (paddr, data),
        // An AMO's result was written by the L1D as part of its atomic
        // access and published at commit; a failed SC wrote nothing.
        StoreResolution::Applied { .. }
        | StoreResolution::Pending
        | StoreResolution::Ready { .. }
        | StoreResolution::Cancelled => return true,
    };

    // Only pure RAM addresses go through the WCB coalesce path. HTIF and
    // other MMIO overlays must bypass it so the per-store MemReq carries the
    // original data to the device (WCB drain packets carry zero data).
    let width_bytes = width_to_bytes(store.width);
    let pure_ram = is_pure_ram(state, paddr, store.width);

    if !state.core.wcb.is_disabled() && pure_ram {
        // Publish now so subsequent loads via the fast path see the new
        // value while the WCB coalesces dirty-line accounting; the WCB
        // drain only signals the line was dirty, it doesn't carry data.
        state.publish_write(paddr, data, store.width);
        for (part_paddr, part_data, part_bytes) in line_parts(state, paddr, data, width_bytes) {
            let evicted = state.core.wcb.merge_store(part_paddr, part_data, part_bytes);
            if evicted.is_none() {
                state.shared.stats.counter(state.core.stat_paths.wcb.coalesces).inc();
            }
            if let Some(drain) = evicted {
                emit_line_writeback(state, common, PhysAddr::new(drain.line_addr));
                state.shared.stats.counter(state.core.stat_paths.wcb.drains).inc();
            }
        }
    } else {
        write_store_to_memory(state, common, paddr, data, store.width);
    }
    trace_commit!(state.config.general.trace_instructions;
        paddr      = %crate::trace::Hex(paddr.val()),
        data       = %crate::trace::Hex(data),
        width      = ?store.width,
        via_wcb    = !state.core.wcb.is_disabled() && pure_ram,
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
pub(crate) fn drain_all_committed(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
    vec_store_buffer: &mut VecStoreBuffer,
) {
    while let Some(store) = store_buffer.drain_one() {
        if let StoreResolution::Committed { paddr, data } = store.resolution {
            write_store_to_memory(state, common, paddr, data, store.width);
        }
    }
    vec_store_buffer.drain_all_committed(state, common);
    flush_wcb(state, common);
}

/// Flushes all WCB entries by emitting write-back `MemReq` packets.
fn flush_wcb(state: &mut CoreCtx<'_>, common: &mut BackendCommon) {
    let drains = state.core.wcb.flush_all();
    for drain in drains {
        emit_line_writeback(state, common, PhysAddr::new(drain.line_addr));
        state.shared.stats.counter(state.core.stat_paths.wcb.drains).inc();
    }
}

/// Emits a dirty-line writeback to the L1D for a line the pipeline drained
/// (a WCB line, or a line a cache-maintenance instruction pushed out). The
/// cache merges it or forwards it down the hierarchy; the memory controller
/// accounts the DRAM write.
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
            op: MemOp::Writeback { dirty: true },
        },
    );
}

/// Writes back every dirty line a cache maintenance operation pushed out.
fn write_back_lines(state: &mut CoreCtx<'_>, common: &mut BackendCommon, lines: &[DirtyLine]) {
    for dirty in lines {
        emit_line_writeback(state, common, dirty.line.phys());
    }
}

/// Performs a CBO on `block`, the physical block memory1 translated, or
/// returns the illegal-instruction trap its gate raises.
fn commit_cbo(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    op: SystemOp,
    block: PhysAddr,
    inst: u32,
) -> Option<Trap> {
    let effect = match cbo::gate(&state.hart.csrs, state.hart.privilege, op, inst) {
        Ok(effect) => effect,
        Err(trap) => return Some(trap),
    };
    let paddr = block.val();
    match effect {
        CboEffect::Zero => cboz_write(state, common, paddr),
        CboEffect::Invalidate => {
            let _ = state.core.l1_d_cache.invalidate_line(paddr);
        }
        CboEffect::Flush => {
            if let Some(dirty) = state.core.l1_d_cache.invalidate_line(paddr) {
                write_back_lines(state, common, &[dirty]);
            }
        }
        CboEffect::Clean => {
            if let Some(dirty) = state.core.l1_d_cache.clean_line(paddr) {
                write_back_lines(state, common, &[dirty]);
            }
        }
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

/// Publishes a store's data and emits its `MemReq` (op = Write).
///
/// Data dimension: for RAM-backed addresses the bytes are published now so
/// subsequent loads via the RAM fast path see the new value. MMIO addresses
/// are not backed by RAM, so the device's `Handle::handle` runs the side
/// effect when the packet reaches it.
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
    if width == MemWidth::Nop {
        return;
    }
    state.publish_write(paddr, data, width);
    emit_store_write_packet(state, common, paddr, data, width);
}

/// Emits the `MemReq`s (op = Write) for a store whose data is already
/// published: the timing side of [`write_store_to_memory`]. A RAM store
/// straddling a cache line is one write per line.
fn emit_store_write_packet(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
) {
    if width == MemWidth::Nop {
        return;
    }
    if !is_pure_ram(state, paddr, width) {
        emit_store_write_packet_to(state, common, paddr, data, width, ComponentId::Bus);
        return;
    }
    let l1d = ComponentId::Cache(common.l1_d_id);
    for (part_paddr, part_data, _) in line_parts(state, paddr, data, width.bytes() as usize) {
        emit_store_write_packet_to(state, common, part_paddr, part_data, width, l1d);
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
    let line_bytes = state.core.l1_d_cache.line_bytes() as u64;
    if !unaligned::crosses_cache_line(paddr.val(), width_bytes as u64, line_bytes) {
        return vec![(paddr, data, width_bytes)];
    }
    let second = (paddr.val() | (line_bytes - 1)) + 1;
    let first_bytes = (second - paddr.val()) as usize;
    vec![
        (paddr, data, first_bytes),
        (PhysAddr::new(second), data >> (8 * first_bytes), width_bytes - first_bytes),
    ]
}

/// Emits one `MemReq` (op = Write) to `target`.
fn emit_store_write_packet_to(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
    target: ComponentId,
) {
    let access_size = match width {
        MemWidth::Byte => AccessSize::B1,
        MemWidth::Half => AccessSize::B2,
        MemWidth::Word => AccessSize::B4,
        MemWidth::Double => AccessSize::B8,
        MemWidth::Nop => return,
    };
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;
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

/// Interrupts in the privileged spec's fixed decreasing priority order (MEI,
/// MSI, MTI, SEI, SSI, STI), as `(mip bit, mie bit, mideleg bit)`.
const INTERRUPT_PRIORITY: [(u64, u64, u64); 6] = [
    (csr::MIP_MEIP, csr::MIE_MEIP, 1 << DELEG_MEIP_BIT),
    (csr::MIP_MSIP, csr::MIE_MSIP, 1 << DELEG_MSIP_BIT),
    (csr::MIP_MTIP, csr::MIE_MTIE, 1 << DELEG_MTIP_BIT),
    (csr::MIP_SEIP, csr::MIE_SEIP, 1 << DELEG_SEIP_BIT),
    (csr::MIP_SSIP, csr::MIE_SSIP, 1 << DELEG_SSIP_BIT),
    (csr::MIP_STIP, csr::MIE_STIE, 1 << DELEG_STIP_BIT),
];

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

    // Interrupts destined for M-mode are taken before any destined for S-mode.
    [false, true].into_iter().find_map(|to_supervisor| {
        INTERRUPT_PRIORITY
            .iter()
            .filter(|&&(_, _, deleg_bit)| {
                (state.hart.csrs.mideleg & deleg_bit != 0) == to_supervisor
            })
            .find_map(|&(bit, enable_bit, deleg_bit)| check(bit, enable_bit, deleg_bit))
    })
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
            state.shared.stats.counter(state.core.stat_paths.commit.fp_load).inc();
        } else {
            state.shared.stats.counter(state.core.stat_paths.commit.op_load).inc();
        }
    } else if entry.ctrl.mem_write {
        if entry.ctrl.rs2_fp {
            state.shared.stats.counter(state.core.stat_paths.commit.fp_store).inc();
        } else {
            state.shared.stats.counter(state.core.stat_paths.commit.op_store).inc();
        }
    } else if matches!(entry.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump) {
        state.shared.stats.counter(state.core.stat_paths.commit.op_branch).inc();
    } else if !matches!(entry.ctrl.system_op, SystemOp::None) {
        state.shared.stats.counter(state.core.stat_paths.commit.op_system).inc();
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
                state.shared.stats.counter(state.core.stat_paths.commit.fp_arith).inc();
            }
            AluOp::FDiv | AluOp::FSqrt => {
                state.shared.stats.counter(state.core.stat_paths.commit.fp_div_sqrt).inc();
            }
            AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => {
                state.shared.stats.counter(state.core.stat_paths.commit.fp_fma).inc();
            }
            _ => state.shared.stats.counter(state.core.stat_paths.commit.op_alu).inc(),
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
            state.shared.stats.counter(state.core.stat_paths.commit.vec_load).inc();
        }
        VectorOp::VStoreUnit
        | VectorOp::VStoreMask
        | VectorOp::VStoreWholeReg
        | VectorOp::VStoreStride
        | VectorOp::VStoreIndexOrd
        | VectorOp::VStoreIndexUnord => {
            state.shared.stats.counter(state.core.stat_paths.commit.vec_store).inc();
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
            state.shared.stats.counter(state.core.stat_paths.commit.vec_int).inc();
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
            state.shared.stats.counter(state.core.stat_paths.commit.vec_fp).inc();
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
            state.shared.stats.counter(state.core.stat_paths.commit.vec_misc).inc();
        }
    }
}

/// Performs selective SFENCE.VMA TLB flushing at commit time per the privileged spec:
/// rs1==0,rs2==0: flush all TLBs;
/// rs1!=0,rs2==0: flush TLB entries matching vaddr in rs1;
/// rs1==0,rs2!=0: flush non-global TLB entries matching ASID in rs2;
/// rs1!=0,rs2!=0: flush TLB entry matching both vaddr and ASID.
fn sfence_vma_commit(state: &mut CoreCtx<'_>, info: &SfenceVmaInfo) {
    match (!info.rs1_idx.is_zero(), !info.rs2_idx.is_zero()) {
        (false, false) => {
            state.core.mmu.dtlb.flush();
            state.core.mmu.itlb.flush();
            state.core.mmu.l2_tlb.flush();
        }
        (true, false) => {
            let vpn = Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
            state.core.mmu.dtlb.flush_vaddr(vpn);
            state.core.mmu.itlb.flush_vaddr(vpn);
            state.core.mmu.l2_tlb.flush_vaddr(vpn);
        }
        (false, true) => {
            let asid = Asid::new(info.rs2_val as u16);
            state.core.mmu.dtlb.flush_asid(asid);
            state.core.mmu.itlb.flush_asid(asid);
            state.core.mmu.l2_tlb.flush_asid(asid);
        }
        (true, true) => {
            let vpn = Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
            let asid = Asid::new(info.rs2_val as u16);
            state.core.mmu.dtlb.flush_vaddr_asid(vpn, asid);
            state.core.mmu.itlb.flush_vaddr_asid(vpn, asid);
            state.core.mmu.l2_tlb.flush_vaddr_asid(vpn, asid);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::InstSize;
    use crate::config::Config;

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
        let state = sys.core_ctx(0);

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
        let state = sys.core_ctx(0);

        state.hart.csrs.mip = csr::MIP_SEIP;
        state.hart.csrs.mie = csr::MIE_SEIP;
        state.hart.csrs.mstatus |= csr::MSTATUS_SIE;
        state.hart.csrs.mideleg |= 1 << DELEG_SEIP_BIT;
        state.hart.privilege = PrivilegeMode::Supervisor;

        assert_eq!(check_interrupts(&state), Some(Trap::SupervisorExternalInterrupt));
    }

    /// Commits one cycle with an older store committed but not yet drained
    /// and an AMO at the ROB head; returns whether the AMO retired.
    fn amo_retires_behind_an_undrained_store(release: bool) -> bool {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);
        let mut store_buffer = StoreBuffer::new(4);
        let mut vec_store_buffer = VecStoreBuffer::new(
            4,
            crate::core::pipeline::vec_store_buffer::VecStoreForwarding::Off,
        );
        let mut scoreboard = Scoreboard::new();
        let older = RobTag(900);
        assert!(store_buffer.allocate(older, MemWidth::Double));
        store_buffer.resolve(
            older,
            crate::common::VirtAddr::new(0x8000_1000),
            PhysAddr::new(0x8000_1000),
            1,
        );
        store_buffer.mark_committed(older);
        let ctrl = crate::core::pipeline::signals::ControlSignals {
            atomic_op: AtomicOp::Swap,
            release,
            mem_read: true,
            mem_write: true,
            width: MemWidth::Double,
            ..Default::default()
        };
        let amo = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(0),
                false,
                ctrl,
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();
        assert!(store_buffer.allocate(amo, MemWidth::Double));
        store_buffer.resolve(
            amo,
            crate::common::VirtAddr::new(0x8000_2000),
            PhysAddr::new(0x8000_2000),
            1,
        );
        rob.complete(amo, 0);

        let mut common = BackendCommon::default();
        let event = commit_stage(
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
        assert!(event.is_none());
        rob.is_empty()
    }

    #[test]
    fn a_release_atomic_waits_for_older_stores_to_drain() {
        assert!(!amo_retires_behind_an_undrained_store(true));
    }

    #[test]
    fn an_atomic_without_release_does_not_wait_for_older_stores() {
        assert!(amo_retires_behind_an_undrained_store(false));
    }

    #[test]
    fn test_commit_stage_normal() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let mut rob = Rob::new(4);
        let mut store_buffer = StoreBuffer::new(4);
        let mut vec_store_buffer = VecStoreBuffer::new(
            4,
            crate::core::pipeline::vec_store_buffer::VecStoreForwarding::Off,
        );
        let mut scoreboard = Scoreboard::new();

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

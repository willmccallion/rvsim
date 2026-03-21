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
use crate::common::{Asid, LrScRecord, RegIdx, SfenceVmaInfo, Trap, Vpn};
use crate::core::Cpu;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::arch::trap::TrapHandler;
use crate::core::cpu::PC_TRACE_MAX;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::PhysRegFile;
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::{Rob, RobState};
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::signals::{AluOp, ControlFlow, MemWidth, SystemOp};
use crate::core::pipeline::store_buffer::{StoreBuffer, StoreResolution, width_to_bytes};
use crate::core::units::bru::BranchPredictor;
use crate::trace_branch;
use crate::trace_commit;
use crate::trace_csr;
use crate::trace_trap;

/// Executes the Commit stage.
///
/// Retires up to `width` instructions from the ROB head per cycle.
/// Handles register writes, CSR application, trap dispatch, and
/// store buffer drain.
#[allow(clippy::too_many_arguments)]
pub fn commit_stage(
    cpu: &mut Cpu,
    rob: &mut Rob,
    store_buffer: &mut StoreBuffer,
    scoreboard: &mut Scoreboard,
    committed_rename_map: &mut RenameMap,
    free_list: &mut FreeList,
    width: usize,
    mut load_queue: Option<&mut LoadQueue>,
    mut prf: Option<&mut PhysRegFile>,
) -> Option<(Trap, u64)> {
    let mut trap_event: Option<(Trap, u64)> = None;

    // Check for interrupts before committing.
    // Always check — even with an empty ROB (e.g., timer fired during a stall
    // with no instructions in-flight). Use cpu.pc as EPC when ROB is empty.
    {
        let epc = if cpu.wfi_waiting {
            cpu.wfi_pc
        } else if let Some(head) = rob.peek_head() {
            head.pc
        } else {
            cpu.committed_next_pc // ROB empty: use last committed PC + size
        };

        let interrupt = check_interrupts(cpu);
        if let Some(interrupt_trap) = interrupt {
            cpu.wfi_waiting = false;
            trace_trap!(cpu.trace;
                event      = "interrupt",
                epc        = %crate::trace::Hex(epc),
                cause      = ?interrupt_trap,
                mip        = %crate::trace::Hex(cpu.csrs.mip),
                mie        = %crate::trace::Hex(cpu.csrs.mie),
                mstatus    = %crate::trace::Hex(cpu.csrs.mstatus),
                priv_mode  = ?cpu.privilege,
                "CM: interrupt detected — flushing pipeline"
            );
            trap_event = Some((interrupt_trap, epc));
        } else if cpu.wfi_waiting {
            // WFI active: never fall through to the commit loop.
            // The ROB may contain wrong-path instructions fetched
            // speculatively past the WFI function (the frontend predicted
            // past the ret and kept fetching). Committing those would
            // corrupt architectural state.
            //
            // If an interrupt is pending & enabled (but not taken as a
            // trap — e.g., delegated to a mode we're not in), this is a
            // non-trap wakeup: resume at wfi_pc and let the caller flush.
            let pending = cpu.csrs.mip;
            let enabled = cpu.csrs.mie;
            if (pending & enabled) != 0 {
                cpu.wfi_waiting = false;
                cpu.pc = cpu.wfi_pc;
                cpu.redirect_pending = true;
            } else {
                cpu.stats.cycles_wfi += 1;
            }
            cpu.stats.retire_histogram[0] += 1;
            return trap_event;
        }
    }

    // If interrupt detected, don't commit — flush everything
    if trap_event.is_some() {
        cpu.stats.retire_histogram[0] += 1;
        return trap_event;
    }

    // Commit up to `width` entries from ROB head
    let mut retired_count: usize = 0;
    let rob_empty_at_start = rob.peek_head().is_none();
    for _ in 0..width {
        let Some(head) = rob.peek_head() else { break };

        // Safety guard: a load must not retire while older stores have unresolved
        // addresses. Without this, a bypassed load's LQ entry gets deallocated
        // before memory2 can detect a violation against a later-resolving store.
        if head.state == RobState::Completed
            && head.ctrl.mem_read
            && store_buffer.has_unresolved_store_before(head.tag)
        {
            break;
        }

        if head.state == RobState::Issued {
            break; // Not ready yet
        }

        if head.state == RobState::Faulted {
            // Synchronous exception: take the trap
            if let Some(entry) = rob.commit_head()
                && let Some(ref the_trap) = entry.trap
            {
                #[cfg(feature = "commit-log")]
                if let Some(ref mut log) = cpu.commit_log {
                    use crate::common::Trap;
                    use std::io::Write;
                    // Log all faulting instructions except fetch-stage page/access
                    // faults (spike doesn't log those because they have no valid
                    // instruction bits). Illegal instructions detected at fetch
                    // ARE logged because they have valid encoding bits.
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
                trace_trap!(cpu.trace;
                    event     = "sync-exception",
                    pc        = %crate::trace::Hex(entry.pc),
                    rob_tag   = entry.tag.0,
                    cause     = ?the_trap,
                    priv_mode = ?cpu.privilege,
                    mstatus   = %crate::trace::Hex(cpu.csrs.mstatus),
                    "CM: synchronous exception at commit"
                );
                // Reclaim the faulting instruction's physical register.
                // The instruction produced no result, so its phys_dst is
                // orphaned — the committed rename map still holds the old
                // mapping (old_phys_dst). The post-trap pipeline flush will
                // reclaim all *remaining* ROB entries' phys_dst, but this
                // entry has already been popped from the ROB, so its
                // phys_dst would leak without this explicit reclaim.
                if entry.phys_dst.0 != 0 {
                    free_list.reclaim(entry.phys_dst);
                }
                trap_event = Some((the_trap.clone(), entry.pc));
            }
            break;
        }

        // SFENCE.VMA serialization barrier: refuse to commit until all
        // committed stores have drained to RAM.  The PTW reads PTEs
        // directly from memory (it cannot see store buffer entries), so
        // if we flush the TLBs while PTE-modifying stores are still in
        // the store buffer, the next page walk would read stale PTEs.
        //
        // We check has_committed_stores() rather than !is_empty() because
        // the store buffer may also contain speculative entries from
        // instructions younger than this SFENCE.VMA — those can never
        // commit (we're blocking the commit loop) and will be squashed
        // by the full pipeline flush after this fence retires.
        if head.ctrl.system_op == SystemOp::SfenceVma && store_buffer.has_committed_stores() {
            break;
        }

        // Completed — retire
        let Some(entry) = rob.commit_head() else { break };
        retired_count += 1;

        // Track the next-to-commit PC for accurate interrupt EPC when ROB is empty.
        // For taken branches and jumps, the next PC is the branch target, not pc+4.
        // Using pc+4 here would cause interrupts arriving during an empty-ROB window
        // (e.g., after a misprediction flush) to save the wrong EPC, resuming
        // execution at the fallthrough address instead of the branch target.
        cpu.committed_next_pc = match entry.ctrl.control_flow {
            ControlFlow::Jump => {
                entry.bp_target.unwrap_or_else(|| entry.pc.wrapping_add(entry.inst_size.as_u64()))
            }
            ControlFlow::Branch if entry.bp_outcome.taken => {
                entry.bp_target.unwrap_or_else(|| entry.pc.wrapping_add(entry.inst_size.as_u64()))
            }
            _ => entry.pc.wrapping_add(entry.inst_size.as_u64()),
        };

        trace_commit!(cpu.trace;
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

        // Write to commit log if enabled (deferred until after register write
        // so we can include the destination register value).
        #[cfg(feature = "commit-log")]
        let commit_log_entry: Option<(u64, u32, bool, usize, u64)> = {
            if cpu.commit_log.is_some() {
                let has_rd =
                    (entry.ctrl.reg_write && !entry.rd.is_zero()) || entry.ctrl.fp_reg_write;
                Some((entry.pc, entry.inst, has_rd, entry.rd.as_usize(), entry.result.unwrap_or(0)))
            } else {
                None
            }
        };

        // Update PC trace
        cpu.pc_trace.push((entry.pc, entry.inst));
        if cpu.pc_trace.len() > PC_TRACE_MAX {
            let _ = cpu.pc_trace.remove(0);
        }

        // Statistics
        if entry.inst != 0 && entry.inst != 0x13 {
            cpu.stats.instructions_retired += 1;
            update_instruction_stats(cpu, &entry);
        }

        // Apply deferred branch predictor update (only update on committed branches)
        if entry.bp_update {
            cpu.branch_predictor.update_branch(
                entry.bp_pc,
                entry.bp_outcome.taken,
                entry.bp_target,
                &entry.bp_ghr_snapshot,
            );
            trace_branch!(cpu.trace;
                event         = "update",
                pc            = %crate::trace::Hex(entry.bp_pc),
                rob_tag       = entry.tag.0,
                actual_taken  = entry.bp_outcome.taken,
                actual_target = %crate::trace::Hex(entry.bp_target.unwrap_or(0)),
                mispredicted  = entry.bp_outcome.mispredicted,
                "CM: branch predictor updated at commit"
            );
            if entry.bp_outcome.mispredicted {
                cpu.stats.committed_branch_mispredictions += 1;
            } else {
                cpu.stats.committed_branch_predictions += 1;
            }
        }

        // Write to register file
        debug_assert!(
            entry.result.is_some() || (!entry.ctrl.reg_write && !entry.ctrl.fp_reg_write),
            "CM: committing instruction with reg_write but no result: rob_tag={} pc={:#x}",
            entry.tag.0, entry.pc,
        );
        let val = entry.result.unwrap_or(0);
        if entry.ctrl.fp_reg_write {
            cpu.regs.write_f(entry.rd, val);
            scoreboard.clear_if_match(entry.rd, true, entry.tag);
            // Update committed rename map and recycle the old physical reg
            if entry.old_phys_dst.0 != entry.phys_dst.0 {
                free_list.reclaim(entry.old_phys_dst);
            }
            committed_rename_map.set(entry.rd, true, entry.phys_dst);
            // Set FS to DIRTY when any FP register is written
            cpu.csrs.mstatus = (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            cpu.csrs.sstatus = (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            trace_commit!(cpu.trace;
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
            cpu.regs.write(entry.rd, val);
            scoreboard.clear_if_match(entry.rd, false, entry.tag);
            // Update committed rename map and recycle the old physical reg
            if entry.old_phys_dst.0 != entry.phys_dst.0 {
                free_list.reclaim(entry.old_phys_dst);
            }
            committed_rename_map.set(entry.rd, false, entry.phys_dst);
            trace_commit!(cpu.trace;
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

        // Write deferred commit log entry (now that rd has been written).
        #[cfg(feature = "commit-log")]
        if let Some((pc, inst, has_rd, rd, val)) = commit_log_entry {
            if let Some(ref mut log) = cpu.commit_log {
                use std::io::Write;
                if has_rd {
                    let _ =
                        writeln!(log, "core   0: 0x{pc:016x} (0x{inst:08x}) x{rd} 0x{val:016x}");
                } else {
                    let _ = writeln!(log, "core   0: 0x{pc:016x} (0x{inst:08x})");
                }
            }
        }

        // Apply deferred PTE A/D bit update.
        // The page table walker defers A/D bit writes until commit to
        // prevent speculative instructions from corrupting page tables.
        if let Some(pte_upd) = entry.pte_update {
            write_store_to_memory(cpu, pte_upd.pte_addr, pte_upd.pte_value, MemWidth::Double);
        }

        // Apply deferred FP exception flags (accumulated during execution).
        // This must happen before CSR writes so that a CSR read of fflags
        // at execute time (which already drained older flags) stays consistent.
        if entry.fp_flags != 0 {
            cpu.csrs.fflags |= entry.fp_flags as u64;
            cpu.csrs.mstatus = (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            cpu.csrs.sstatus = (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
        }

        // Apply deferred CSR write
        if let Some(csr_update) = entry.csr_update {
            // SATP writes change the address translation mode. All preceding
            // stores (page table setup, etc.) must be visible in physical
            // memory before the new page tables are consulted. Drain the
            // entire store buffer so the PTW reads up-to-date PTEs.
            if csr_update.addr == csr::SATP {
                drain_all_committed(cpu, store_buffer);
            }
            // For the O3 backend, fflags/fcsr CSR writes are applied eagerly at
            // complete time (in step 6a of tick()) to avoid races with younger
            // speculative FP instructions. Skip re-applying them here.
            if !csr_update.applied {
                cpu.csr_write(csr_update.addr, csr_update.new_val);
            }
            trace_csr!(cpu.trace;
                op       = if csr_update.applied { "write-eager" } else { "write-deferred" },
                pc       = %crate::trace::Hex(entry.pc),
                rob_tag  = entry.tag.0,
                csr_addr = %crate::trace::Hex32(csr_update.addr.as_u32()),
                old_val  = %crate::trace::Hex(csr_update.old_val),
                new_val  = %crate::trace::Hex(csr_update.new_val),
                deferred = !csr_update.applied,
                "CM: CSR write applied at commit"
            );
            // SATP changes address translation: any instructions fetched
            // between the execute-stage redirect and this commit used the
            // old page tables. Force a re-flush so the frontend re-fetches
            // with the new translation context.
            //
            // We must also reset cpu.pc to the instruction after this CSR,
            // because Fetch1 has been advancing cpu.pc since the execute-stage
            // redirect. Without this, the frontend would restart from the
            // stale (advanced) cpu.pc, skipping instructions.
            if csr_update.addr == csr::SATP {
                cpu.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
                cpu.redirect_pending = true;
            }
            // CSR instructions are serializing — drain before committing more
            break;
        }

        // Handle MRET/SRET at commit (serializing instructions)
        if entry.ctrl.system_op == SystemOp::Mret {
            cpu.do_mret();
            cpu.committed_next_pc = cpu.pc;
            trace_trap!(cpu.trace;
                event      = "return",
                insn       = "MRET",
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::trace::Hex(cpu.pc),
                mstatus    = %crate::trace::Hex(cpu.csrs.mstatus),
                priv_mode  = ?cpu.privilege,
                "CM: MRET committed — privilege restored"
            );
            break;
        }
        if entry.ctrl.system_op == SystemOp::Sret {
            cpu.do_sret();
            cpu.committed_next_pc = cpu.pc;
            trace_trap!(cpu.trace;
                event      = "return",
                insn       = "SRET",
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::trace::Hex(cpu.pc),
                mstatus    = %crate::trace::Hex(cpu.csrs.mstatus),
                priv_mode  = ?cpu.privilege,
                "CM: SRET committed — privilege restored"
            );
            break;
        }

        // WFI — applied at commit so it is properly ordered with
        // preceding instructions in the same fetch group.
        if entry.ctrl.system_op == SystemOp::Wfi {
            if cpu.csrs.mie != 0 || cpu.csrs.mip != 0 {
                // At least one interrupt source is enabled or pending —
                // enter the waiting state.  The interrupt check at the
                // top of commit_instructions will wake us.
                cpu.wfi_waiting = true;
                cpu.wfi_pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            } else {
                // Nothing enabled, nothing pending — NOP (advance past WFI
                // to avoid deadlock, e.g. OpenSBI early boot).
                cpu.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
                cpu.redirect_pending = true;
            }
            cpu.committed_next_pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            break;
        }

        // Apply deferred LR/SC reservation action.
        //
        // LR/SC reservation checks are deferred from Memory2 (speculative) to
        // commit (architectural) so that squashed instructions cannot corrupt
        // the reservation state.  See ISSUES.md Finding 5.
        if let Some(lr_sc_rec) = entry.lr_sc {
            match lr_sc_rec {
                LrScRecord::Lr { paddr } => {
                    cpu.set_reservation(paddr);
                }
                LrScRecord::Sc { paddr } => {
                    if cpu.check_reservation(paddr) {
                        // SC success — reservation valid, clear it and let
                        // the store (already in store buffer) drain normally.
                        cpu.clear_reservation();
                    } else {
                        // SC failure — reservation was invalid.  The Memory2
                        // stage optimistically assumed success (rd=0, store
                        // resolved).  We must undo this:
                        // 1. Cancel the store buffer entry (no memory write).
                        // 2. Write rd = 1 (failure) to the register file.
                        // 3. Redirect the pipeline to re-fetch from the next
                        //    instruction.  Younger instructions that consumed
                        //    rd=0 are stale and must be discarded.
                        store_buffer.cancel(entry.tag);
                        if entry.ctrl.reg_write && !entry.rd.is_zero() {
                            cpu.regs.write(entry.rd, 1);
                            // Also fix the PRF so the post-flush rename map
                            // sees the corrected value (rd=1, not the
                            // optimistic rd=0 written at writeback).
                            if let Some(ref mut prf) = prf {
                                prf.write(entry.phys_dst, 1);
                            }
                        }
                        cpu.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
                        cpu.redirect_pending = true;
                        break;
                    }
                }
            }
        }

        // Mark store buffer entry as committed (for stores)
        if entry.ctrl.mem_write {
            // Per RISC-V spec Section 8.2: a store to the reservation set
            // between a paired LR and SC must cause the SC to fail.  Clear
            // the reservation when a non-LR/SC store (regular store or AMO)
            // commits to an address in the reservation granule.
            //
            // SC stores are excluded: they already handle the reservation
            // above via LrScRecord::Sc.
            if entry.lr_sc.is_none()
                && let Some(paddr) = store_buffer.find_paddr(entry.tag)
                && cpu.check_reservation(paddr)
            {
                cpu.clear_reservation();
            }
            store_buffer.mark_committed(entry.tag);
        }

        // Deallocate load queue entry (for loads)
        if entry.ctrl.mem_read
            && let Some(ref mut lq) = load_queue
        {
            lq.deallocate(entry.tag);
        }

        // FENCE.I always drains all committed stores — FENCE.I must see
        // prior stores before refilling I-cache.
        // SFENCE.VMA does NOT need drain_all_committed here because the
        // stall check above already guarantees the store buffer is empty.
        // FENCE: only drain when pred.w is set (older stores must be globally
        // visible before younger succ operations proceed).
        if entry.ctrl.system_op == SystemOp::FenceI {
            drain_all_committed(cpu, store_buffer);
            // FENCE.I: flush I-cache AFTER store drain so refills see new data.
            // The execute stage already redirected the frontend; this flush
            // ensures the I-cache doesn't hold stale lines when fetching resumes.
            let _ = cpu.l1_i_cache.invalidate_all();
            // Re-redirect the frontend: the execute-time redirect may have
            // already caused fetches with stale I-cache data. Force a new
            // redirect so the frontend re-fetches with the flushed I-cache.
            cpu.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            cpu.redirect_pending = true;
            // FENCE.I is serializing — stop committing so the redirect
            // takes effect before any younger instructions retire.
            // Without this break, younger instructions (fetched before the
            // store drain) could commit in the same cycle with stale data.
            break;
        } else if entry.ctrl.system_op == SystemOp::Fence {
            let pred_bits = ((entry.inst >> 24) & 0xF) as u8;
            let pred_w = pred_bits & 0b0001 != 0;
            let pred_r = pred_bits & 0b0010 != 0;
            // FENCE pred,succ:
            // - pred.w: drain store buffer (older stores globally visible)
            // - pred.r: older loads already completed by commit order
            // - Both pred.r and pred.w: full drain + flush WCB
            if pred_w || pred_r {
                drain_all_committed(cpu, store_buffer);
            }
        }

        // SFENCE.VMA: the store buffer is guaranteed empty (stall above).
        // Flush TLBs with proper ASID/vaddr granularity, clear the
        // reservation, and trigger a full pipeline squash so that all
        // younger instructions (which may have fetched with stale TLBs)
        // are discarded and re-fetched with the now-clean translations.
        if let Some(info) = entry.sfence_vma {
            sfence_vma_commit(cpu, &info);
            cpu.clear_reservation();
            cpu.pc = entry.pc.wrapping_add(entry.inst_size.as_u64());
            cpu.redirect_pending = true;
            break;
        }

        // Ensure x0 stays zero
        cpu.regs.write(RegIdx::new(0), 0);
    }

    // Record retirement histogram and ROB-empty tracking
    if retired_count == 0 && rob_empty_at_start {
        cpu.stats.cycles_rob_empty += 1;
    }
    cpu.stats.retire_histogram[retired_count.min(3)] += 1;

    // Drain one committed store to memory per cycle
    drain_one_store(cpu, store_buffer);

    trap_event
}

/// Writes a single committed store from the store buffer to memory.
///
/// If a Write Combining Buffer (WCB) is configured, stores are first merged
/// into the WCB. The WCB coalesces stores to the same cache line and only
/// drains to L1D when an entry is evicted (LRU) or flushed.
fn drain_one_store(cpu: &mut Cpu, store_buffer: &mut StoreBuffer) {
    let Some(store) = store_buffer.drain_one() else { return };
    let StoreResolution::Committed { paddr, data } = store.resolution else {
        // Cancelled (failed SC) — no write needed, just drain the slot.
        return;
    };

    let is_ram = paddr.val() >= cpu.ram_start && paddr.val() < cpu.ram_end;
    let width_bytes = width_to_bytes(store.width);

    if !cpu.wcb.is_disabled() && is_ram {
        // Merge into WCB; if an entry was evicted, drain it through cache
        let evicted = cpu.wcb.merge_store(paddr, data, width_bytes);
        if evicted.is_none() {
            // Store absorbed by WCB (coalesced or allocated new entry)
            cpu.stats.wcb_coalesces += 1;
        }
        if let Some(drain) = evicted {
            // Evicted WCB entry: simulate cache write for the evicted line
            let addr = crate::common::PhysAddr::new(drain.line_addr);
            let _latency = cpu.simulate_memory_access(addr, crate::common::AccessType::Write);
            cpu.stats.wcb_drains += 1;
        }
    } else {
        // No WCB or MMIO: direct cache access + memory write
        if is_ram {
            let _latency = cpu.simulate_memory_access(paddr, crate::common::AccessType::Write);
        }
    }
    // Always write the actual data to memory (WCB is timing-only)
    write_store_to_memory(cpu, paddr, data, store.width);
    trace_commit!(cpu.trace;
        paddr      = %crate::trace::Hex(paddr.val()),
        data       = %crate::trace::Hex(data),
        width      = ?store.width,
        via_wcb    = !cpu.wcb.is_disabled(),
        "CM: committed store drained to memory"
    );
}

/// Drains **all** committed stores from the store buffer to memory.
///
/// Called before SATP writes to ensure page table entries set up by
/// preceding stores are visible in physical memory before the page
/// table walker consults them. Also flushes the WCB.
fn drain_all_committed(cpu: &mut Cpu, store_buffer: &mut StoreBuffer) {
    while let Some(store) = store_buffer.drain_one() {
        if let StoreResolution::Committed { paddr, data } = store.resolution {
            let is_ram = paddr.val() >= cpu.ram_start && paddr.val() < cpu.ram_end;
            if is_ram {
                let _latency = cpu.simulate_memory_access(paddr, crate::common::AccessType::Write);
            }
            write_store_to_memory(cpu, paddr, data, store.width);
        }
    }
    // Flush remaining WCB entries through the cache hierarchy
    flush_wcb(cpu);
}

/// Flushes all WCB entries through the cache hierarchy.
fn flush_wcb(cpu: &mut Cpu) {
    let drains = cpu.wcb.flush_all();
    for drain in drains {
        let addr = crate::common::PhysAddr::new(drain.line_addr);
        let _latency = cpu.simulate_memory_access(addr, crate::common::AccessType::Write);
        cpu.stats.wcb_drains += 1;
    }
}

/// Writes a store's data to the correct memory target (RAM fast-path or bus).
fn write_store_to_memory(
    cpu: &mut Cpu,
    paddr: crate::common::PhysAddr,
    data: u64,
    width: MemWidth,
) {
    let raw = paddr.val();
    let in_htif = cpu.htif_range.is_some_and(|(lo, hi)| raw >= lo && raw < hi);
    let is_ram = !in_htif && raw >= cpu.ram_start && raw < cpu.ram_end;
    if is_ram {
        let offset = (raw - cpu.ram_start) as usize;
        unsafe {
            match width {
                MemWidth::Byte => *cpu.ram_ptr.add(offset) = data as u8,
                MemWidth::Half => {
                    (cpu.ram_ptr.add(offset) as *mut u16).write_unaligned(data as u16);
                }
                MemWidth::Word => {
                    (cpu.ram_ptr.add(offset) as *mut u32).write_unaligned(data as u32);
                }
                MemWidth::Double => (cpu.ram_ptr.add(offset) as *mut u64).write_unaligned(data),
                MemWidth::Nop => {}
            }
        }
    } else {
        match width {
            MemWidth::Byte => cpu.bus.bus.write_u8(paddr, data as u8),
            MemWidth::Half => cpu.bus.bus.write_u16(paddr, data as u16),
            MemWidth::Word => cpu.bus.bus.write_u32(paddr, data as u32),
            MemWidth::Double => cpu.bus.bus.write_u64(paddr, data),
            MemWidth::Nop => {}
        }
    }
}

/// Checks for pending interrupts. Returns the trap if one should be taken.
fn check_interrupts(cpu: &Cpu) -> Option<Trap> {
    let mip = cpu.csrs.mip;
    let mie = cpu.csrs.mie;
    let mstatus = cpu.csrs.mstatus;

    let m_global_ie = (mstatus & csr::MSTATUS_MIE) != 0;
    let s_global_ie = (mstatus & csr::MSTATUS_SIE) != 0;

    let check = |bit: u64, enable_bit: u64, deleg_bit: u64| -> Option<Trap> {
        let pending = (mip & bit) != 0;
        let enabled = (mie & enable_bit) != 0;
        if !pending || !enabled {
            return None;
        }

        let delegated = (cpu.csrs.mideleg & deleg_bit) != 0;
        let target_priv =
            if delegated { PrivilegeMode::Supervisor } else { PrivilegeMode::Machine };

        if cpu.privilege.to_u8() < target_priv.to_u8() {
            return Some(TrapHandler::irq_to_trap(bit));
        }
        if cpu.privilege == target_priv {
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
const fn update_instruction_stats(cpu: &mut Cpu, entry: &crate::core::pipeline::rob::RobEntry) {
    if entry.ctrl.mem_read {
        if entry.ctrl.fp_reg_write {
            cpu.stats.inst_fp_load += 1;
        } else {
            cpu.stats.inst_load += 1;
        }
    } else if entry.ctrl.mem_write {
        if entry.ctrl.rs2_fp {
            cpu.stats.inst_fp_store += 1;
        } else {
            cpu.stats.inst_store += 1;
        }
    } else if matches!(entry.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump) {
        cpu.stats.inst_branch += 1;
    } else if !matches!(entry.ctrl.system_op, SystemOp::None) {
        cpu.stats.inst_system += 1;
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
            | AluOp::FMvToX
            | AluOp::FMvToF => cpu.stats.inst_fp_arith += 1,
            AluOp::FDiv | AluOp::FSqrt => cpu.stats.inst_fp_div_sqrt += 1,
            AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => {
                cpu.stats.inst_fp_fma += 1;
            }
            _ => cpu.stats.inst_alu += 1,
        }
    }
}

/// Performs selective SFENCE.VMA TLB/cache flushing at commit time.
///
/// Uses the deferred operand values captured at execute time for proper
/// ASID/vaddr granularity, matching the RISC-V privileged specification:
///
/// * rs1 == 0, rs2 == 0: flush all TLB entries + D-cache + I-cache
/// * rs1 != 0, rs2 == 0: flush TLB entries matching virtual address in rs1
/// * rs1 == 0, rs2 != 0: flush non-global TLB entries matching ASID in rs2
/// * rs1 != 0, rs2 != 0: flush TLB entry matching both vaddr and ASID
fn sfence_vma_commit(cpu: &mut Cpu, info: &SfenceVmaInfo) {
    match (!info.rs1_idx.is_zero(), !info.rs2_idx.is_zero()) {
        (false, false) => {
            cpu.mmu.dtlb.flush();
            cpu.mmu.itlb.flush();
            cpu.mmu.l2_tlb.flush();
            let _ = cpu.l1_d_cache.flush();
            let _ = cpu.l1_i_cache.invalidate_all();
        }
        (true, false) => {
            let vpn = Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
            cpu.mmu.dtlb.flush_vaddr(vpn);
            cpu.mmu.itlb.flush_vaddr(vpn);
            cpu.mmu.l2_tlb.flush_vaddr(vpn);
        }
        (false, true) => {
            let asid = Asid::new(info.rs2_val as u16);
            cpu.mmu.dtlb.flush_asid(asid);
            cpu.mmu.itlb.flush_asid(asid);
            cpu.mmu.l2_tlb.flush_asid(asid);
        }
        (true, true) => {
            let vpn = Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
            let asid = Asid::new(info.rs2_val as u16);
            cpu.mmu.dtlb.flush_vaddr_asid(vpn, asid);
            cpu.mmu.itlb.flush_vaddr_asid(vpn, asid);
            cpu.mmu.l2_tlb.flush_vaddr_asid(vpn, asid);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::InstSize;
    use crate::config::Config;
    use crate::core::Cpu;
    use crate::soc::builder::System;

    #[test]
    fn test_check_interrupts_none() {
        let config = Config::default();
        let system = System::new(&config, "");
        let cpu = Cpu::new(system, &config);

        assert!(check_interrupts(&cpu).is_none());
    }

    #[test]
    fn test_check_interrupts_m_mode() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csrs.mip = csr::MIP_MEIP;
        cpu.csrs.mie = csr::MIE_MEIP;
        cpu.csrs.mstatus |= csr::MSTATUS_MIE;
        cpu.privilege = PrivilegeMode::Machine;

        assert_eq!(check_interrupts(&cpu), Some(Trap::MachineExternalInterrupt));
    }

    #[test]
    fn test_check_interrupts_s_mode_delegated() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csrs.mip = csr::MIP_SEIP;
        cpu.csrs.mie = csr::MIE_SEIP;
        cpu.csrs.mstatus |= csr::MSTATUS_SIE;
        cpu.csrs.mideleg |= 1 << DELEG_SEIP_BIT;
        cpu.privilege = PrivilegeMode::Supervisor;

        assert_eq!(check_interrupts(&cpu), Some(Trap::SupervisorExternalInterrupt));
    }

    #[test]
    fn test_commit_stage_normal() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

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

        let trap = commit_stage(
            &mut cpu,
            &mut rob,
            &mut store_buffer,
            &mut scoreboard,
            &mut committed_rename_map,
            &mut free_list,
            1,
            None,
            None,
        );
        assert!(trap.is_none());
        assert_eq!(cpu.regs.read(RegIdx::new(1)), 42);
    }
}

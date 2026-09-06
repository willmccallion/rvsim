//! Memory1 stage: translation, alignment, and `MemReq` issuance.
//!
//! For each `ExMem1Entry` coming in from execute, memory1:
//!
//! - **Trap propagation** — the entry already carries a trap: pass it
//!   straight through to the M1→M2 latch so writeback can mark the ROB
//!   entry faulted at its normal stage.
//! - **Non-memory ops** — pass through to M1→M2; memory2 / writeback handle
//!   the ALU pass-through.
//! - **Memory ops** — perform alignment + load/store trigger checks, then
//!   translate the virtual address.
//!   - On a [`TranslateResult::NeedPte`], park the entry under an
//!     [`OutstandingWalk`] with [`WalkContinuation::LoadStore`] and emit a
//!     `MemReq` for the PTE. When the walk completes the mailbox drain
//!     re-injects the `ExMem1Entry` here.
//!   - On a fault (PMP / page fault / unmapped paddr), emit a trapped
//!     `Mem1Mem2Entry`.
//!   - For demand **loads**: check store-buffer forwarding first.
//!     - SB hit → push directly to M1→M2 with `load_data` filled and
//!       `sb_forwarded = true`. No `MemReq` issued.
//!     - SB partial overlap → stall (push back to input).
//!     - SB miss → emit `MemReq` to L1D and park [`OutstandingLoad`].
//!   - For **AMO / LR**: stall on any older store to the same address still
//!     resident in the SB. Otherwise emit `MemReq` and park.
//!   - For **stores**: pass to M1→M2 with the resolved `paddr`. Memory2
//!     resolves the store buffer and checks for ordering violations.
//!   - For **SC**: same as stores, plus an `AtomicOp::Sc` marker so memory2
//!     records the deferred `LrScRecord::Sc`.

use crate::common::{
    AccessType, ExceptionStage, PhysAddr, PteUpdate, Trap, VirtAddr,
};
use crate::sim::SimState;
use crate::core::arch::mode::PrivilegeMode;
use crate::sim::state::memory::TranslateResult;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry};
use crate::core::pipeline::outstanding::{OutstandingLoad, OutstandingWalk, WalkContinuation};
use crate::core::pipeline::signals::{AtomicOp, MemWidth};
use crate::core::pipeline::store_buffer::ForwardResult;
use crate::core::units::lsu::unaligned;
use crate::sim::components::ComponentId;
use crate::sim::packet::{self, AccessSize, MemOp, Packet};

/// Outcome of processing a single `ExMem1Entry`.
enum EntryOutcome {
    /// Entry was passed downstream (`Mem1Mem2` push); continue iterating.
    Done,
    /// SB partial overlap or atomic-vs-SB stall — push back to input.
    Stall(ExMem1Entry),
    /// Entry was parked on a page-table walk in the engine's outstanding
    /// tables. Younger memory ops cannot pass an unresolved older
    /// translation in an in-order pipeline, so halt iteration and push any
    /// remaining entries back to the input latch.
    ParkedWalk,
}

/// Executes the Memory1 stage. Returns immediately; ordering-violation
/// detection happens at Memory2 once stores have actually resolved their
/// store-buffer slots.
pub fn memory1_stage<E: ExecutionEngine>(
    cpu: &mut SimState,
    engine: &mut E,
    input: &mut Vec<ExMem1Entry>,
) {
    let mut entries = std::mem::take(input);
    // Out-of-order execute can drop entries into execute_mem1 in completion
    // order rather than program order. memory1's SB-forward / atomic-vs-SB
    // checks only inspect *older* store entries, so a younger load that
    // ends up at the front of `entries` can permanently stall on a store
    // that hasn't committed yet, even though the older load it sits behind
    // would have drained its blocking store first. Sort by rob_tag so the
    // oldest in-flight memory op gets first crack at the SB each cycle.
    entries.sort_by_key(|e| e.rob_tag.0);
    let mut iter = entries.into_iter();

    while let Some(ex) = iter.next() {
        match process_entry(cpu, engine, ex) {
            EntryOutcome::Done => {}
            EntryOutcome::Stall(ex) => {
                input.push(ex);
                input.extend(iter);
                return;
            }
            EntryOutcome::ParkedWalk => {
                input.extend(iter);
                return;
            }
        }
    }
}

fn process_entry<E: ExecutionEngine>(
    cpu: &mut SimState,
    engine: &mut E,
    ex: ExMem1Entry,
) -> EntryOutcome {
    // 1. Trap propagation.
    if ex.trap.is_some() {
        push_passthrough_with_trap(engine, ex);
        return EntryOutcome::Done;
    }

    let needs_translation = ex.ctrl.mem_read || ex.ctrl.mem_write;
    if !needs_translation {
        push_passthrough(engine, ex);
        return EntryOutcome::Done;
    }

    // 2. Alignment.
    let size = unaligned::width_to_bytes(ex.ctrl.width);
    let is_atomic = ex.ctrl.atomic_op != AtomicOp::None;
    if !unaligned::is_aligned(ex.alu, size)
        && (cpu.config.memory.misaligned_access_trap || is_atomic)
    {
        let trap = if ex.ctrl.mem_write {
            unaligned::store_misaligned_trap(ex.alu)
        } else {
            unaligned::load_misaligned_trap(ex.alu)
        };
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    // 3. Sdtrig load/store triggers.
    if ex.ctrl.mem_read && !is_atomic && cpu.check_load_trigger(ex.alu) {
        let trap = Trap::Breakpoint(ex.pc);
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }
    if ex.ctrl.mem_write && !is_atomic && cpu.check_store_trigger(ex.alu) {
        let trap = Trap::Breakpoint(ex.pc);
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    // 4. Translation.
    let access_type = if ex.ctrl.mem_write { AccessType::Write } else { AccessType::Read };
    let outcome = cpu.translate(VirtAddr::new(ex.alu), access_type, size);
    let (paddr, pte_update) = match outcome {
        TranslateResult::Ready(r) => {
            if let Some(trap) = r.trap {
                push_trap(engine, ex, trap, ExceptionStage::Memory);
                return EntryOutcome::Done;
            }
            (r.paddr, r.pte_update)
        }
        TranslateResult::NeedPte { pte_addr, state } => {
            park_walk(cpu, engine, state, pte_addr, ex);
            return EntryOutcome::ParkedWalk;
        }
    };

    // 5. S/U-mode access fault on unmapped paddr; M-mode firmware can probe.
    if cpu.hart.privilege != PrivilegeMode::Machine && !cpu.bus.is_valid_address(paddr) {
        let trap = if ex.ctrl.mem_write {
            Trap::StoreAccessFault(ex.alu)
        } else {
            Trap::LoadAccessFault(ex.alu)
        };
        push_trap(engine, ex, trap, ExceptionStage::Memory);
        return EntryOutcome::Done;
    }

    // 6. Load-queue address fill (O3).
    if ex.ctrl.mem_read
        && let Some(lq) = engine.load_queue_mut()
    {
        let elem = ex.vec_mem.as_ref().map(|v| v.elem_idx);
        lq.fill_address(ex.rob_tag, elem, VirtAddr::new(ex.alu), paddr);
    }

    // 7. Operation dispatch.
    let vaddr = VirtAddr::new(ex.alu);

    if ex.ctrl.mem_write && !is_atomic {
        push_resolved_store(engine, ex, paddr, vaddr, pte_update);
        return EntryOutcome::Done;
    }

    if is_atomic {
        if ex.ctrl.atomic_op == AtomicOp::Sc {
            push_resolved_sc(engine, ex, paddr, vaddr, pte_update);
            return EntryOutcome::Done;
        }
        // LR / AMO: stall on older stores to this address.
        if engine
            .store_buffer()
            .has_older_store_to(paddr, ex.ctrl.width, ex.rob_tag)
        {
            return EntryOutcome::Stall(ex);
        }
        emit_load_req(cpu, engine, ex, paddr, vaddr, pte_update, true);
        return EntryOutcome::Done;
    }

    // Demand load: try store-buffer forwarding first.
    match engine
        .store_buffer()
        .forward_load(paddr, ex.ctrl.width, ex.rob_tag)
    {
        ForwardResult::Hit(raw_val) => {
            push_sb_forwarded_load(engine, ex, paddr, vaddr, pte_update, raw_val);
            EntryOutcome::Done
        }
        ForwardResult::Stall => EntryOutcome::Stall(ex),
        ForwardResult::Miss => {
            emit_load_req(cpu, engine, ex, paddr, vaddr, pte_update, false);
            EntryOutcome::Done
        }
    }
}

/// Pushes an ALU/non-memory entry directly into the M1→M2 latch.
fn push_passthrough<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry) {
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: ex.rob_tag,
        pc: ex.pc,
        inst: ex.inst,
        inst_size: ex.inst_size,
        rd: ex.rd,
        rd_phys: ex.rd_phys,
        alu: ex.alu,
        vaddr: VirtAddr::new(0),
        paddr: PhysAddr::new(0),
        store_data: ex.store_data,
        load_data: 0,
        sb_forwarded: false,
        ctrl: ex.ctrl,
        trap: None,
        exception_stage: None,
        fp_flags: ex.fp_flags,
        complete_cycle: 0,
        pte_update: None,
        sfence_vma: ex.sfence_vma,
        vec_mem: ex.vec_mem,
    });
}

/// Forwards an entry that already carries a trap from an earlier stage.
fn push_passthrough_with_trap<E: ExecutionEngine>(engine: &mut E, ex: ExMem1Entry) {
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: ex.rob_tag,
        pc: ex.pc,
        inst: ex.inst,
        inst_size: ex.inst_size,
        rd: ex.rd,
        rd_phys: ex.rd_phys,
        alu: ex.alu,
        vaddr: VirtAddr::new(ex.alu),
        paddr: PhysAddr::new(0),
        store_data: ex.store_data,
        load_data: 0,
        sb_forwarded: false,
        ctrl: ex.ctrl,
        trap: ex.trap,
        exception_stage: ex.exception_stage,
        fp_flags: ex.fp_flags,
        complete_cycle: 0,
        pte_update: None,
        sfence_vma: ex.sfence_vma,
        vec_mem: ex.vec_mem,
    });
}

/// Emits a fresh trap entry into the M1→M2 latch.
fn push_trap<E: ExecutionEngine>(
    engine: &mut E,
    ex: ExMem1Entry,
    trap: Trap,
    stage: ExceptionStage,
) {
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: ex.rob_tag,
        pc: ex.pc,
        inst: ex.inst,
        inst_size: ex.inst_size,
        rd: ex.rd,
        rd_phys: ex.rd_phys,
        alu: ex.alu,
        vaddr: VirtAddr::new(ex.alu),
        paddr: PhysAddr::new(0),
        store_data: ex.store_data,
        load_data: 0,
        sb_forwarded: false,
        ctrl: ex.ctrl,
        trap: Some(trap),
        exception_stage: Some(stage),
        fp_flags: ex.fp_flags,
        complete_cycle: 0,
        pte_update: None,
        sfence_vma: ex.sfence_vma,
        vec_mem: ex.vec_mem,
    });
}

/// Pushes a resolved store entry into the M1→M2 latch. Memory2 resolves the
/// store buffer slot and checks the load queue for ordering violations.
fn push_resolved_store<E: ExecutionEngine>(
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    pte_update: Option<PteUpdate>,
) {
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: ex.rob_tag,
        pc: ex.pc,
        inst: ex.inst,
        inst_size: ex.inst_size,
        rd: ex.rd,
        rd_phys: ex.rd_phys,
        alu: ex.alu,
        vaddr,
        paddr,
        store_data: ex.store_data,
        load_data: 0,
        sb_forwarded: false,
        ctrl: ex.ctrl,
        trap: None,
        exception_stage: None,
        fp_flags: ex.fp_flags,
        complete_cycle: 0,
        pte_update,
        sfence_vma: ex.sfence_vma,
        vec_mem: ex.vec_mem,
    });
}

/// Pushes a resolved store-conditional entry into the M1→M2 latch.
fn push_resolved_sc<E: ExecutionEngine>(
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    pte_update: Option<PteUpdate>,
) {
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: ex.rob_tag,
        pc: ex.pc,
        inst: ex.inst,
        inst_size: ex.inst_size,
        rd: ex.rd,
        rd_phys: ex.rd_phys,
        alu: ex.alu,
        vaddr,
        paddr,
        store_data: ex.store_data,
        load_data: 0,
        sb_forwarded: false,
        ctrl: ex.ctrl,
        trap: None,
        exception_stage: None,
        fp_flags: ex.fp_flags,
        complete_cycle: 0,
        pte_update,
        sfence_vma: ex.sfence_vma,
        vec_mem: ex.vec_mem,
    });
}

/// Pushes an SB-forwarded load into M1→M2 with the forwarded raw value
/// already in `load_data`.
fn push_sb_forwarded_load<E: ExecutionEngine>(
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    pte_update: Option<PteUpdate>,
    raw_val: u64,
) {
    engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: ex.rob_tag,
        pc: ex.pc,
        inst: ex.inst,
        inst_size: ex.inst_size,
        rd: ex.rd,
        rd_phys: ex.rd_phys,
        alu: ex.alu,
        vaddr,
        paddr,
        store_data: ex.store_data,
        load_data: raw_val,
        sb_forwarded: true,
        ctrl: ex.ctrl,
        trap: None,
        exception_stage: None,
        fp_flags: ex.fp_flags,
        complete_cycle: 0,
        pte_update,
        sfence_vma: ex.sfence_vma,
        vec_mem: ex.vec_mem,
    });
}

/// Issues a `MemReq` for a load / LR / AMO and parks the entry.
fn emit_load_req<E: ExecutionEngine>(
    cpu: &mut SimState,
    engine: &mut E,
    ex: ExMem1Entry,
    paddr: PhysAddr,
    vaddr: VirtAddr,
    pte_update: Option<PteUpdate>,
    is_atomic: bool,
) {
    let access_size = match ex.ctrl.width {
        MemWidth::Byte => AccessSize::B1,
        MemWidth::Half => AccessSize::B2,
        MemWidth::Word => AccessSize::B4,
        MemWidth::Double | MemWidth::Nop => AccessSize::B8,
    };
    let op = if is_atomic {
        let packet_atomic = match ex.ctrl.atomic_op {
            AtomicOp::Lr => packet::AtomicOp::Lr,
            AtomicOp::Swap => packet::AtomicOp::Swap,
            AtomicOp::Add => packet::AtomicOp::Add,
            AtomicOp::Xor => packet::AtomicOp::Xor,
            AtomicOp::And => packet::AtomicOp::And,
            AtomicOp::Or => packet::AtomicOp::Or,
            AtomicOp::Min => packet::AtomicOp::Min,
            AtomicOp::Max => packet::AtomicOp::Max,
            AtomicOp::Minu => packet::AtomicOp::MinU,
            AtomicOp::Maxu => packet::AtomicOp::MaxU,
            AtomicOp::Sc => unreachable!("Sc is resolved at memory1 before emit"),
            AtomicOp::None => unreachable!("is_atomic checked"),
        };
        MemOp::Atomic { op: packet_atomic, data: ex.store_data }
    } else {
        MemOp::Read
    };

    let target = mmio_or_l1d(cpu, engine, paddr, access_size);
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;

    let cycle = cpu.cycle;
    cpu.event_queue.schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq { req_id, paddr, vaddr: Some(vaddr), size: access_size, op },
    );

    let _ = engine.common_mut().outstanding_loads.insert(
        req_id,
        OutstandingLoad { entry: ex, paddr, vaddr, pte_update },
    );
}

/// Records the parked walk and issues the PTE `MemReq`.
fn park_walk<E: ExecutionEngine>(
    cpu: &mut SimState,
    engine: &mut E,
    state: crate::core::units::mmu::ptw::WalkState,
    pte_addr: PhysAddr,
    ex: ExMem1Entry,
) {
    let common = engine.common_mut();
    let req_id = common.alloc_req_id();
    let l1_d_id = common.l1_d_id;
    let pipeline_id = common.pipeline_id;
    let _ = common.outstanding_walks.insert(
        req_id,
        OutstandingWalk {
            state,
            pte_addr,
            continuation: WalkContinuation::LoadStore(ex),
        },
    );

    let cycle = cpu.cycle;
    cpu.event_queue.schedule(
        cycle,
        ComponentId::Cache(l1_d_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: pte_addr,
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Read,
        },
    );
}

/// Returns `ComponentId::Bus` when `paddr` is MMIO and `ComponentId::Cache(L1D)`
/// when it's pure RAM. MMIO loads/stores must bypass the L1D — caching MMIO
/// makes subsequent accesses hit the cache and silently miss the device's
/// side effect (e.g. an HTIF tohost write that L1D hit would never reach the
/// device).
fn mmio_or_l1d<E: ExecutionEngine>(
    cpu: &SimState,
    engine: &E,
    paddr: PhysAddr,
    size: AccessSize,
) -> ComponentId {
    let size_bytes = match size {
        AccessSize::B1 => 1u64,
        AccessSize::B2 => 2,
        AccessSize::B4 => 4,
        AccessSize::B8 => 8,
        AccessSize::Line => 64,
    };
    if cpu.bus.ram_region_for(paddr.val(), size_bytes).is_some() {
        ComponentId::Cache(engine.common().l1_d_id)
    } else {
        ComponentId::Bus
    }
}

//! Rename for the out-of-order engine: physical registers from the free
//! list, the speculative rename map, and a ROB, load-queue, store-buffer
//! and checkpoint slot per instruction.
//!
//! Source physical registers are captured before the destination is
//! renamed, so an instruction reading its own destination (`addi x5, x5,
//! 16`) sees the previous producer.

use super::{O3Engine, RenameView};
use crate::exec::compute::vector::mem::{
    is_vec_load, is_vec_store, vec_mem_dst_count, vec_mem_emul_regs,
};
use crate::exec::inst::Inst;
use crate::exec::signals::{ControlFlow, ControlSignals};
use crate::isa::op::VecOperandGroups;
use crate::isa::rvv::{VRegIdx, VectorConfig, VectorRounding, parse_vtype};
use crate::trace_rename;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::vec_mem::expected_load_micro_ops;
use crate::uarch::pipeline::engine::{ExecutionEngine, Renamed};
use crate::uarch::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::free_list::FreeList;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rob::RobTag;
use crate::uarch::pipeline::trace::Stage;

impl O3Engine {
    /// Renames one decoded instruction and allocates its backend slots.
    /// Returns the instruction when a slot it needs is short, before
    /// anything is allocated; `can_accept` covers only the ROB and issue
    /// queue slots every instruction needs, as gem5's rename checks each
    /// instruction's own resources.
    pub(super) fn rename_one(&mut self, state: &mut StageCtx<'_>, id: IdExEntry) -> Renamed {
        if !self.serialization.admits(state.cycle) {
            state.counter(state.core().stat_paths.pipeline.stalls_serialize).inc();
            return Renamed::Stalled(Box::new(id));
        }
        let csrs = &state.hart().csrs;
        let vector =
            VectorContext { config: self.vector_config(csrs), rounding: csrs.vector_rounding() };
        let load_slots = self.load_slots_for(&id, &vector.config, state);
        let checkpoint = if self.needs_checkpoint(&id.inst.ctrl) {
            let Some(slot) = self.checkpoints.free_slot() else {
                state.counter(state.core().stat_paths.pipeline.stalls_checkpoint).inc();
                return Renamed::Stalled(Box::new(id));
            };
            Some(slot)
        } else {
            None
        };
        if !self.backend_slots().has_room_for(&id, load_slots) {
            return Renamed::Stalled(Box::new(id));
        }
        let groups = vector_operand_groups(&id.inst.ctrl, &vector.config);
        let sources = self.capture_sources(&id.inst, groups);
        let Some(dests) = self.allocate_destinations(&id, groups.vd) else {
            return Renamed::Stalled(Box::new(id));
        };
        state.trace_stage(id.seq, Stage::Rename);
        self.allocate_memory_slots(dests.rob_tag, &id.inst.ctrl, load_slots);
        // After the destinations: the checkpoint holds the branch's own rd.
        if let Some(slot) = checkpoint {
            let checkpoint_id = slot.fill(&mut self.checkpoints, dests.rob_tag, &self.rename_map);
            self.rob.set_checkpoint_id(dests.rob_tag, checkpoint_id);
        }
        let entry = renamed_entry(id, &sources, &dests, vector);
        trace_renamed(state, &entry, dests.old_phys_dst);
        if entry.inst.ctrl.system_op.serializes_after() {
            self.serialization = super::serialize::Serialization::after(entry.rob_tag);
        }
        Renamed::Accepted(Box::new(entry))
    }

    /// Whether a branch or jump takes a rename-map checkpoint for its
    /// recovery.
    const fn needs_checkpoint(&self, ctrl: &ControlSignals) -> bool {
        matches!(ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump)
            && self.checkpoints.capacity() > 0
    }

    const fn backend_slots(&self) -> BackendSlots<'_> {
        BackendSlots {
            store_buffer: &self.store_buffer,
            vec_store_buffer: &self.vec_store_buffer,
            load_queue: &self.load_queue,
            free_list: &self.free_list,
            view: self.rename_view,
        }
    }

    /// The physical registers `inst` reads, mapped before its destinations
    /// are renamed: a masked compare writing v0 is masked by the old v0.
    fn capture_sources(&self, inst: &Inst, groups: VecOperandGroups) -> Sources {
        let ctrl = &inst.ctrl;
        let map = &self.rename_map;
        let is_vector = ctrl.vec_lmul_regs > 0;
        let vs1_count = if is_vector { groups.vs1 } else { 0 };
        let vs2_count = if is_vector { groups.vs2 } else { 0 };
        let reads_old_vd = is_vector && (ctrl.vec_reg_write || is_vec_store(ctrl.vec_op));
        let vs3_count = if reads_old_vd { groups.vd } else { 0 };
        Sources {
            rs1: map.get(ctrl.src1(inst.rs1)),
            rs2: map.get(ctrl.src2(inst.rs2)),
            rs3: ctrl.src3(inst.rs3).map_or(PhysReg(0), |rs3| map.get(rs3)),
            vs1: map.vec_group(ctrl.vs1, vs1_count),
            vs2: map.vec_group(ctrl.vs2, vs2_count),
            vs3: map.vec_group(ctrl.vd, vs3_count),
            vs1_count,
            vs2_count,
            vs3_count,
            mask: ctrl.is_masked_vector_op().then(|| map.get_vec(VRegIdx::new(0))),
        }
    }

    /// Allocates the physical registers and ROB entry `id` writes and
    /// renames its destinations to them, its vector group `vd_count`
    /// registers long; `None` when a vector register or the ROB entry is
    /// short, with nothing left allocated.
    fn allocate_destinations(&mut self, id: &IdExEntry, vd_count: u8) -> Option<Destinations> {
        let inst = &id.inst;
        // x0 stays unrenamed: hardwired zero, never freed at commit.
        let dest = inst.ctrl.dest(inst.rd);
        let (rd_phys, old_phys_dst) = match dest {
            Some(dest) => (self.free_list.allocate()?, self.rename_map.get(dest)),
            None => (PhysReg(0), PhysReg(0)),
        };
        let vd_count = if inst.ctrl.vec_reg_write { vd_count } else { 0 };
        let vd_regs = self.vec_free_list.reserve(vd_count as usize);
        let rob_tag = vd_regs.as_ref().and_then(|_| {
            let rob = &mut self.rob;
            rob.allocate(
                inst.pc,
                inst.bits,
                inst.size,
                inst.rd,
                inst.ctrl,
                rd_phys,
                old_phys_dst,
                id.seq,
            )
        });
        let (Some(vd_regs), Some(rob_tag)) = (vd_regs, rob_tag) else {
            if dest.is_some() {
                self.free_list.reclaim(rd_phys);
            }
            return None;
        };
        let mut vd_phys = [VecPhysReg::ZERO; 8];
        for (slot, reg) in vd_phys.iter_mut().zip(vd_regs.take()) {
            *slot = reg;
        }
        if let Some(dest) = dest {
            self.rename_map.set(dest, rd_phys);
            self.prf.allocate(rd_phys);
        }
        if vd_count > 0 {
            let renamed = &vd_phys[..vd_count as usize];
            let vec_old_phys = self.rename_map.rename_vec_group(inst.ctrl.vd, renamed);
            for &reg in renamed {
                self.vec_prf.allocate(reg);
            }
            self.rob.set_vec_phys_dst(rob_tag, vd_phys, vec_old_phys, vd_count);
        }
        Some(Destinations { rob_tag, rd_phys, old_phys_dst, vd_phys })
    }
    /// Takes `ctrl`'s store-buffer or load-queue slots for ROB entry
    /// `rob_tag`, which `has_room_for` saw free.
    fn allocate_memory_slots(
        &mut self,
        rob_tag: RobTag,
        ctrl: &ControlSignals,
        load_slots: LoadSlots,
    ) {
        let store_slot_allocated = if ctrl.uses_store_buffer() {
            self.store_buffer.allocate(rob_tag, ctrl.width)
        } else if is_vec_store(ctrl.vec_op) {
            self.vec_store_buffer.allocate(rob_tag)
        } else {
            true
        };
        let load_slot_allocated = match load_slots {
            LoadSlots::None => true,
            LoadSlots::Scalar => self.load_queue.allocate(rob_tag, ctrl.width.bytes() as usize),
            LoadSlots::Vector(count) => self.load_queue.reserve(rob_tag, count),
        };
        debug_assert!(
            store_slot_allocated && load_slot_allocated,
            "has_room_for checked the memory slots"
        );
        if ctrl.uses_store_buffer() {
            self.rename_view.sq -= 1;
            self.stores_renamed_this_cycle += 1;
        }
        self.rename_view.lq -= load_slots.count();
    }
}

/// The vector CSR state an instruction executes with, taken at rename so a
/// `vsetvl` still in flight cannot change it.
#[derive(Clone, Copy)]
struct VectorContext {
    config: VectorConfig,
    rounding: VectorRounding,
}

/// The physical registers an instruction reads, as rename found them.
struct Sources {
    rs1: PhysReg,
    rs2: PhysReg,
    rs3: PhysReg,
    vs1: [VecPhysReg; 8],
    vs2: [VecPhysReg; 8],
    /// The old destination group, for tail and mask merging and as store data.
    vs3: [VecPhysReg; 8],
    vs1_count: u8,
    vs2_count: u8,
    vs3_count: u8,
    mask: Option<VecPhysReg>,
}

/// The ROB entry and physical registers rename allocated for an
/// instruction's results.
struct Destinations {
    rob_tag: RobTag,
    rd_phys: PhysReg,
    /// The register `rd` mapped to before, freed when this one commits.
    old_phys_dst: PhysReg,
    vd_phys: [VecPhysReg; 8],
}

/// The issue-queue entry for `id`, renamed to `sources` and `dests`.
const fn renamed_entry(
    id: IdExEntry,
    sources: &Sources,
    dests: &Destinations,
    vector: VectorContext,
) -> RenameIssueEntry {
    RenameIssueEntry {
        inst: Inst { rv1: 0, rv2: 0, rv3: 0, ..id.inst },
        rob_tag: dests.rob_tag,
        rs1_phys: sources.rs1,
        rs2_phys: sources.rs2,
        rs3_phys: sources.rs3,
        rd_phys: dests.rd_phys,
        rs1_tag: None,
        rs2_tag: None,
        rs3_tag: None,
        trap: id.trap,
        predicted_target: id.predicted_target,
        seq: id.seq,
        vs1_phys: sources.vs1,
        vs2_phys: sources.vs2,
        vs3_phys: sources.vs3,
        vd_phys: dests.vd_phys,
        vec_src1_count: sources.vs1_count,
        vec_src2_count: sources.vs2_count,
        vec_src3_count: sources.vs3_count,
        mask: sources.mask,
        vector_config: vector.config,
        vector_rounding: vector.rounding,
    }
}

fn trace_renamed(state: &StageCtx<'_>, entry: &RenameIssueEntry, old_phys_dst: PhysReg) {
    trace_rename!(state.config.general.trace_instructions;
        pc         = %crate::common::trace::Hex(entry.inst.pc.val()),
        rob_tag    = %entry.rob_tag,
        rd         = entry.inst.rd.as_usize(),
        rd_phys    = entry.rd_phys.0,
        old_phys   = old_phys_dst.0,
        rs1        = entry.inst.rs1.as_usize(),
        rs1_phys   = entry.rs1_phys.0,
        rs2        = entry.inst.rs2.as_usize(),
        rs2_phys   = entry.rs2_phys.0,
        is_store   = entry.inst.ctrl.mem_write,
        is_load    = entry.inst.ctrl.mem_read,
        is_fp      = entry.inst.ctrl.fp_reg_write,
        "RN: O3 rename"
    );
}

/// The load-queue slots an instruction takes at rename.
#[derive(Clone, Copy)]
enum LoadSlots {
    /// None: it reads no memory.
    None,
    /// One, for a scalar load.
    Scalar,
    /// A vector load's reservation, which its micro-ops claim.
    Vector(usize),
}

impl LoadSlots {
    const fn count(self) -> usize {
        match self {
            Self::None => 0,
            Self::Scalar => 1,
            Self::Vector(count) => count,
        }
    }
}

impl O3Engine {
    /// The load-queue slots `id` takes, renamed under `vector`: a vector
    /// load reserves one per micro-op it expects, at most the whole queue.
    fn load_slots_for(
        &self,
        id: &IdExEntry,
        vector: &VectorConfig,
        state: &StageCtx<'_>,
    ) -> LoadSlots {
        if !is_vec_load(id.inst.ctrl.vec_op) {
            return if id.inst.ctrl.mem_read { LoadSlots::Scalar } else { LoadSlots::None };
        }
        let pipeline = &state.config.pipeline;
        let expected = expected_load_micro_ops(
            &id.inst.ctrl,
            vector,
            pipeline.vlen,
            pipeline.vector_mem_width_bytes(),
        );
        LoadSlots::Vector(expected.min(self.load_queue.capacity()))
    }
}

/// The backend structures an instruction needs a slot in besides the ROB
/// and issue queue `can_accept` covers.
struct BackendSlots<'a> {
    store_buffer: &'a StoreBuffer,
    vec_store_buffer: &'a VecStoreBuffer,
    load_queue: &'a LoadQueue,
    free_list: &'a FreeList<PhysReg>,
    /// The load-queue and store-buffer slots rename sees this cycle.
    view: RenameView,
}

impl BackendSlots<'_> {
    /// True when `id` can take a physical register for a destination, and
    /// a store-buffer, vector-store-buffer or load-queue slot for a memory
    /// op, as gem5's rename checks each instruction's own resources.
    fn has_room_for(&self, id: &IdExEntry, load_slots: LoadSlots) -> bool {
        let needs_dst = id.inst.ctrl.dest(id.inst.rd).is_some();
        let store_slot = if id.inst.ctrl.uses_store_buffer() {
            self.view.sq > 0 && !self.store_buffer.is_full()
        } else if is_vec_store(id.inst.ctrl.vec_op) {
            self.vec_store_buffer.free_slots() > 0
        } else {
            true
        };
        let needed = load_slots.count();
        let load_slot = needed <= self.view.lq && needed <= self.load_queue.free_slots();
        let register = !needs_dst || self.free_list.available() > 0;
        store_slot && load_slot && register
    }
}

/// The register-group sizes of a vector instruction's operands. A vector
/// memory instruction's data and index groups depend on its EEW against
/// the current SEW and LMUL, which `operand_groups` doesn't see, so they
/// are sized here.
pub(super) fn vector_operand_groups(
    ctrl: &ControlSignals,
    vector: &VectorConfig,
) -> VecOperandGroups {
    let mut groups = ctrl.vec_op.operand_groups(
        ctrl.vec_lmul_regs,
        ctrl.vec_lmul_is_fractional,
        ctrl.vec_src_encoding,
        ctrl.vec_nf,
        ctrl.vec_broadcast_vs2,
    );
    let is_mem = is_vec_load(ctrl.vec_op) || is_vec_store(ctrl.vec_op);
    let vtype = parse_vtype(vector.vtype);
    if is_mem && (!vtype.vill || !ctrl.vec_op.depends_on_vtype()) {
        groups.vd =
            vec_mem_dst_count(ctrl.vec_op, ctrl.vec_eew, vtype.vsew, vtype.vlmul, ctrl.vec_nf);
        groups.vs2 = vec_mem_emul_regs(ctrl.vec_op, ctrl.vec_eew, vtype.vsew, vtype.vlmul).1;
    }
    groups
}

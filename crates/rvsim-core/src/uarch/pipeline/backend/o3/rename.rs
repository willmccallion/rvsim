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
use crate::isa::rvv::MAX_GROUP_REGS;
use crate::isa::rvv::{VRegIdx, VectorConfig, VectorRounding, parse_vtype};
use crate::trace_rename;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::vec_mem::expected_load_micro_ops;
use crate::uarch::pipeline::engine::{ExecutionEngine, Renamed};
use crate::uarch::pipeline::latches::{
    IdExEntry, IssueRegisters, RenameIssueEntry, SourceReadiness,
};
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::free_list::FreeList;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rob::RenamedDst;
use crate::uarch::pipeline::rob::RobTag;
use crate::uarch::pipeline::trace::Stage;

impl O3Engine {
    /// Renames one decoded instruction and allocates its backend slots.
    /// Returns the instruction when a slot it needs is short, before
    /// anything is allocated; `can_accept` covers only the ROB and issue
    /// queue slots every instruction needs, as gem5's rename checks each
    /// instruction's own resources.
    pub(super) fn rename_one(
        &mut self,
        state: &mut StageCtx<'_>,
        decoded: IdExEntry,
    ) -> Renamed<RenamedRegisters> {
        if !self.serialization.admits(state.cycle) {
            state.counter(state.core().stat_paths.pipeline.stalls_serialize).inc();
            return Renamed::Stalled(Box::new(decoded));
        }
        let csrs = &state.hart().csrs;
        let vector =
            VectorContext { config: self.vector_config(csrs), rounding: csrs.vector_rounding() };
        let load_slots = self.load_slots_for(&decoded, &vector.config, state);
        let checkpoint = if self.needs_checkpoint(&decoded.inst.ctrl) {
            let Some(slot) = self.checkpoints.free_slot() else {
                state.counter(state.core().stat_paths.pipeline.stalls_checkpoint).inc();
                return Renamed::Stalled(Box::new(decoded));
            };
            Some(slot)
        } else {
            None
        };
        if !self.backend_slots().has_room_for(&decoded, load_slots) {
            return Renamed::Stalled(Box::new(decoded));
        }
        let groups = vector_operand_groups(&decoded.inst.ctrl, &vector.config);
        let sources = self.capture_sources(&decoded.inst, groups);
        let Some(dests) = self.allocate_destinations(&decoded, groups.vd) else {
            return Renamed::Stalled(Box::new(decoded));
        };
        state.trace_stage(decoded.seq, Stage::Rename);
        self.allocate_memory_slots(dests.rob_tag, &decoded.inst.ctrl, load_slots);
        // After the destinations: the checkpoint holds the branch's own rd.
        if let Some(slot) = checkpoint {
            let checkpoint_id = slot.fill(&mut self.checkpoints, dests.rob_tag, &self.rename_map);
            self.rob.set_checkpoint_id(dests.rob_tag, checkpoint_id);
        }
        let entry = renamed_entry(decoded, sources, &dests, vector);
        trace_renamed(state, &entry, dests.renamed.old);
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
    /// The destinations are left for `allocate_destinations`.
    fn capture_sources(&self, inst: &Inst, groups: VecOperandGroups) -> RenamedRegisters {
        let ctrl = &inst.ctrl;
        let map = &self.rename_map;
        let is_vector = ctrl.vec_lmul_regs > 0;
        let vs1_count = if is_vector { groups.vs1 } else { 0 };
        let vs2_count = if is_vector { groups.vs2 } else { 0 };
        let reads_old_vd = is_vector && (ctrl.vec_reg_write || is_vec_store(ctrl.vec_op));
        let vs3_count = if reads_old_vd { groups.vd } else { 0 };
        RenamedRegisters {
            rs1_phys: map.get(ctrl.src1(inst.rs1)),
            rs2_phys: map.get(ctrl.src2(inst.rs2)),
            rs3_phys: ctrl.src3(inst.rs3).map_or(PhysReg::ZERO, |rs3| map.get(rs3)),
            vs1_phys: map.vec_group(ctrl.vs1, vs1_count),
            vs2_phys: map.vec_group(ctrl.vs2, vs2_count),
            vs3_phys: map.vec_group(ctrl.vd, vs3_count),
            vec_src1_count: vs1_count,
            vec_src2_count: vs2_count,
            vec_src3_count: vs3_count,
            mask: ctrl.is_masked_vector_op().then(|| map.get_vec(VRegIdx::new(0))),
            ..RenamedRegisters::default()
        }
    }

    /// Allocates the physical registers and ROB entry `id` writes and
    /// renames its destinations to them, its vector group `vd_count`
    /// registers long; `None` when a vector register or the ROB entry is
    /// short, with nothing left allocated.
    fn allocate_destinations(&mut self, decoded: &IdExEntry, vd_count: u8) -> Option<Destinations> {
        let inst = &decoded.inst;
        // x0 stays unrenamed: hardwired zero, never freed at commit.
        let dest = inst.ctrl.dest(inst.rd);
        let renamed = match dest {
            Some(dest) => {
                RenamedDst { phys: self.free_list.allocate()?, old: self.rename_map.get(dest) }
            }
            None => RenamedDst::NONE,
        };
        let vd_count = if inst.ctrl.vec_reg_write { vd_count } else { 0 };
        let vd_regs = self.vec_free_list.reserve(vd_count as usize);
        let rob_tag = vd_regs.as_ref().and_then(|_| {
            let rob = &mut self.rob;
            rob.allocate(inst, decoded.seq, renamed)
        });
        let (Some(vd_regs), Some(rob_tag)) = (vd_regs, rob_tag) else {
            if dest.is_some() {
                self.free_list.reclaim(renamed.phys);
            }
            return None;
        };
        let mut vd_phys = [VecPhysReg::ZERO; MAX_GROUP_REGS];
        for (slot, reg) in vd_phys.iter_mut().zip(vd_regs.take()) {
            *slot = reg;
        }
        if let Some(dest) = dest {
            self.rename_map.set(dest, renamed.phys);
            self.prf.allocate(renamed.phys);
        }
        if vd_count > 0 {
            let renamed = &vd_phys[..vd_count as usize];
            let vec_old_phys = self.rename_map.rename_vec_group(inst.ctrl.vd, renamed);
            for &reg in renamed {
                self.vec_prf.allocate(reg);
            }
            self.rob.set_vec_phys_dst(rob_tag, vd_phys, vec_old_phys, vd_count);
        }
        Some(Destinations { rob_tag, renamed, vd_phys })
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

/// The ROB entry and physical registers rename allocated for an
/// instruction's results.
struct Destinations {
    rob_tag: RobTag,
    renamed: RenamedDst,
    vd_phys: [VecPhysReg; MAX_GROUP_REGS],
}

/// The issue-queue entry for `decoded`, its sources renamed to `sources`
/// and its results to `dests`.
const fn renamed_entry(
    decoded: IdExEntry,
    sources: RenamedRegisters,
    dests: &Destinations,
    vector: VectorContext,
) -> O3Entry {
    RenameIssueEntry {
        inst: Inst { rv1: 0, rv2: 0, rv3: 0, ..decoded.inst },
        rob_tag: dests.rob_tag,
        trap: decoded.trap,
        predicted_target: decoded.predicted_target,
        seq: decoded.seq,
        vector_config: vector.config,
        vector_rounding: vector.rounding,
        regs: RenamedRegisters { rd_phys: dests.renamed.phys, vd_phys: dests.vd_phys, ..sources },
    }
}

/// The out-of-order backend's registers: the physical registers each
/// source was renamed to, and those the results go to.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenamedRegisters {
    /// rs1's physical register.
    pub rs1_phys: PhysReg,
    /// rs2's physical register.
    pub rs2_phys: PhysReg,
    /// rs3's physical register, x0's when the instruction has no rs3.
    pub rs3_phys: PhysReg,
    /// The physical register the result goes to, x0's when it has none.
    pub rd_phys: PhysReg,
    /// The vs1 group.
    pub vs1_phys: [VecPhysReg; MAX_GROUP_REGS],
    /// The vs2 group.
    pub vs2_phys: [VecPhysReg; MAX_GROUP_REGS],
    /// The old destination group, for tail and mask merging and as store
    /// data.
    pub vs3_phys: [VecPhysReg; MAX_GROUP_REGS],
    /// The destination group.
    pub vd_phys: [VecPhysReg; MAX_GROUP_REGS],
    /// Registers in the vs1 group.
    pub vec_src1_count: u8,
    /// Registers in the vs2 group.
    pub vec_src2_count: u8,
    /// Registers in the vs3 group.
    pub vec_src3_count: u8,
    /// The physical register holding v0 when a masked vector op was renamed.
    pub mask: Option<VecPhysReg>,
}

impl IssueRegisters for RenamedRegisters {
    fn rd_phys(&self) -> PhysReg {
        self.rd_phys
    }

    /// Both known: an O3 instruction waits on physical registers instead.
    fn readiness(&self) -> SourceReadiness {
        SourceReadiness { rs1: true, rs2: true }
    }
}

/// An instruction the out-of-order backend renamed.
pub type O3Entry = RenameIssueEntry<RenamedRegisters>;

fn trace_renamed(state: &StageCtx<'_>, entry: &O3Entry, old_phys_dst: PhysReg) {
    trace_rename!(state.config.general.trace_instructions;
        pc         = %crate::common::trace::Hex(entry.inst.pc.val()),
        rob_tag    = %entry.rob_tag,
        rd         = entry.inst.rd.as_usize(),
        rd_phys    = entry.regs.rd_phys.index(),
        old_phys   = old_phys_dst.index(),
        rs1        = entry.inst.rs1.as_usize(),
        rs1_phys   = entry.regs.rs1_phys.index(),
        rs2        = entry.inst.rs2.as_usize(),
        rs2_phys   = entry.regs.rs2_phys.index(),
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
        decoded: &IdExEntry,
        vector: &VectorConfig,
        state: &StageCtx<'_>,
    ) -> LoadSlots {
        if !is_vec_load(decoded.inst.ctrl.vec_op) {
            return if decoded.inst.ctrl.mem_read { LoadSlots::Scalar } else { LoadSlots::None };
        }
        let pipeline = &state.config.pipeline;
        let expected = expected_load_micro_ops(
            &decoded.inst.ctrl,
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
    fn has_room_for(&self, decoded: &IdExEntry, load_slots: LoadSlots) -> bool {
        let needs_dst = decoded.inst.ctrl.dest(decoded.inst.rd).is_some();
        let store_slot = if decoded.inst.ctrl.uses_store_buffer() {
            self.view.sq > 0 && !self.store_buffer.is_full()
        } else if is_vec_store(decoded.inst.ctrl.vec_op) {
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

//! Commit Stage: retire instructions from ROB head.
//!
//! This stage retires the oldest instruction(s) from the ROB in program order:
//! 1. Write results to the register file.
//! 2. Apply deferred CSR writes.
//! 3. Mark store buffer entries as Committed.
//! 4. Handle traps/interrupts.
//! 5. Drain one committed store to memory per cycle.

mod gate;
mod retire;
mod stats;
mod writes;

pub(crate) use writes::{committed_writes_pending, send_one_write};

use crate::arch::regs::vpr::Vpr;
use crate::common::PhysAddr;
use crate::isa::op::MemWidth;
use crate::isa::privileged::Trap;
use crate::isa::rvv::VRegIdx;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::engine::BackendCommon;
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::checkpoint::{CheckpointId, CheckpointTable};
use crate::uarch::pipeline::rename::free_list::FreeList;
use crate::uarch::pipeline::rename::map::RenameMap;
use crate::uarch::pipeline::rename::prf::{PhysReg, PhysRegFile};
use crate::uarch::pipeline::rename::scoreboard::Scoreboard;
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysRegFile;
use crate::uarch::pipeline::rob::{Rob, RobEntry, RobTag};
use gate::{gate_head, trap_or_interrupt};
use retire::{RetireTargets, retire_entry};

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

/// Whether retirement goes on this cycle.
enum CommitFlow {
    /// Commit goes on to the next step or instruction.
    Continue,
    /// Commit stops for this cycle, with the event it ends in, if any.
    Stop(Option<CommitEvent>),
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
    let now = state.cycle;
    common.deliver_commit_notices(&mut state.core.branch_predictor, now);
    if let CommitFlow::Stop(event) = trap_or_interrupt(state, common, rob) {
        return event;
    }

    let mut event = None;
    let mut retired_count: usize = 0;
    let mut youngest_retired = None;
    let rob_empty_at_start = rob.peek_head().is_none();
    for _ in 0..width {
        let gate = gate_head(state, common, rob, store_buffer, vec_store_buffer, &mut registers);
        if let CommitFlow::Stop(stop) = gate {
            event = stop;
            break;
        }
        let Some(entry) = rob.commit_head() else { break };
        retired_count += 1;
        youngest_retired = Some(entry.seq);
        let mut targets =
            RetireTargets { common, store_buffer, vec_store_buffer, registers: &mut registers };
        if let CommitFlow::Stop(stop) = retire_entry(state, &mut targets, &entry) {
            event = stop;
            break;
        }
    }

    if let Some(seq) = youngest_retired {
        common.note_committed(seq, state.cycle);
    }
    count_retire_width(state, retired_count, rob_empty_at_start);
    send_one_write(state, common, store_buffer, vec_store_buffer);
    event
}

/// Counts how many instructions retired this cycle, and a cycle the ROB
/// had nothing to retire.
fn count_retire_width(state: &mut CoreCtx<'_>, retired: usize, rob_empty_at_start: bool) {
    let paths = &state.core.stat_paths;
    if retired == 0 && rob_empty_at_start {
        state.uncore.stats.counter(paths.pipeline.cycles_rob_empty).inc();
    }
    let bucket = match retired.min(3) {
        0 => paths.commit.retire_hist_zero,
        1 => paths.commit.retire_hist_one,
        2 => paths.commit.retire_hist_two,
        _ => paths.commit.retire_hist_three_plus,
    };
    state.uncore.stats.counter(bucket).inc();
}

/// True when `[paddr, paddr + width)` is RAM with no MMIO overlay, i.e. a
/// write there can be published directly rather than through a device.
fn is_pure_ram(state: &CoreCtx<'_>, paddr: PhysAddr, width: MemWidth) -> bool {
    state.bus.ram_region_for(paddr.val(), width.bytes()).is_some()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::isa::instruction::InstSize;
    use crate::isa::reg::RegIdx;

    #[test]
    fn test_commit_stage_normal() {
        let config = Config::default();
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let mut rob = Rob::new(4);
        let mut store_buffer = StoreBuffer::new(4);
        let mut vec_store_buffer = VecStoreBuffer::new(4, crate::config::VecStoreForwarding::Off);
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

//! Stat paths for every component, allocated once when the component is
//! built.
//!
//! Each component holds a struct of interned [`StatId`]s (`core3.commit.op.load`
//! and so on) so the hot path increments a counter by index and a typo at a
//! writer site is a compile error. Paths are interned once when the component
//! is built.
//!
//! Path grammar: `<subject>.<subsystem>[.<sub>].<counter>`. See
//! `docs/architecture/stats.md` for rationale.

use crate::common::{CoreId, HartId};

use super::StatId;

macro_rules! stat_paths {
    ($(#[$meta:meta])* $name:ident { $( $(#[$fmeta:meta])* $field:ident: $tail:literal ),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug)]
        pub struct $name {
            $( $(#[$fmeta])* pub $field: StatId, )*
        }

        impl $name {
            fn under(subject: &str) -> Self {
                Self { $( $field: StatId::of(&format!("{subject}.{}", $tail)), )* }
            }
        }
    };
}

stat_paths! {
    /// Architectural, per-hart counters (retired insts, traps, mode-cycle mix).
    HartPaths {
        /// Instructions retired (mirrors `Hart::instructions_retired`).
        retired_insts: "retired_insts",
        /// Trap-taken events.
        traps: "traps",
        /// Cycles spent in user (U) privilege.
        cycles_user: "cycles.user",
        /// Cycles spent in supervisor (S) privilege.
        cycles_kernel: "cycles.kernel",
        /// Cycles spent in machine (M) privilege.
        cycles_machine: "cycles.machine",
    }
}

impl HartPaths {
    /// Paths under `hart<N>`.
    #[must_use]
    pub fn new(hart: HartId) -> Self {
        Self::under(&format!("hart{}", hart.val()))
    }
}

stat_paths! {
    /// Commit-stage counters — instruction-mix breakdown at retirement.
    CommitPaths {
        /// Integer load retired.
        op_load: "commit.op.load",
        /// Integer store retired.
        op_store: "commit.op.store",
        /// Branch/jump retired.
        op_branch: "commit.op.branch",
        /// Integer ALU retired.
        op_alu: "commit.op.alu",
        /// System / CSR / ECALL retired.
        op_system: "commit.op.system",
        /// LR, SC or AMO retired.
        op_atomic: "commit.op.atomic",
        /// FP load retired.
        fp_load: "commit.fp.load",
        /// FP store retired.
        fp_store: "commit.fp.store",
        /// FP arithmetic retired.
        fp_arith: "commit.fp.arith",
        /// FP fused multiply-add retired.
        fp_fma: "commit.fp.fma",
        /// FP divide/sqrt retired.
        fp_div_sqrt: "commit.fp.div_sqrt",
        /// Vector integer op retired.
        vec_int: "commit.vec.int",
        /// Vector FP op retired.
        vec_fp: "commit.vec.fp",
        /// Vector load retired.
        vec_load: "commit.vec.load",
        /// Vector store retired.
        vec_store: "commit.vec.store",
        /// Vector misc (permute/mask/config) retired.
        vec_misc: "commit.vec.misc",
        /// Vector crypto (Zvk*) retired.
        vec_crypto: "commit.vec.crypto",
        /// Retire histogram: cycles where 0 insts retired.
        retire_hist_zero: "commit.retire_histogram.zero",
        /// Retire histogram: cycles where exactly 1 inst retired.
        retire_hist_one: "commit.retire_histogram.one",
        /// Retire histogram: cycles where exactly 2 insts retired.
        retire_hist_two: "commit.retire_histogram.two",
        /// Retire histogram: cycles where 3 or more insts retired.
        retire_hist_three_plus: "commit.retire_histogram.three_plus",
    }
}

stat_paths! {
    /// Cross-stage pipeline counters (cycle categories, stalls, flushes).
    PipelinePaths {
        /// Cycles the core has been ticked.
        cycles_total: "pipeline.cycles.total",
        /// Cycles the core spent in WFI (waiting for interrupt).
        cycles_wfi: "pipeline.cycles.wfi",
        /// Cycles the ROB was empty.
        cycles_rob_empty: "pipeline.cycles.rob_empty",
        /// Cycles from a backend redirect (squash, trap, re-execution) until
        /// rename hands on the first instruction from the new path.
        stalls_control: "pipeline.stalls.control",
        /// Fetch1 idle because an earlier fetch group is still waiting on
        /// the I-cache response or an instruction-fetch page walk.
        stalls_fetch_wait: "pipeline.stalls.fetch_wait",
        /// Issue stalled on data hazard (source not ready).
        stalls_data: "pipeline.stalls.data",
        /// Issue stalled on FU structural hazard.
        stalls_fu_structural: "pipeline.stalls.fu_structural",
        /// Issue held queued work for program order.
        stalls_ordering: "pipeline.stalls.ordering",
        /// Downstream backpressure stalls.
        stalls_backpressure: "pipeline.stalls.backpressure",
        /// Dispatch stall (rename → issue queue).
        stalls_dispatch: "pipeline.stalls.dispatch",
        /// Checkpoint allocation stalls.
        stalls_checkpoint: "pipeline.stalls.checkpoint",
        /// Rename held behind a serializing instruction until the ROB drains.
        stalls_serialize: "pipeline.stalls.serialize",
        /// Squash-recovery cycles.
        stalls_squash: "pipeline.stalls.squash",
        /// Total pipeline flushes.
        flushes_total: "pipeline.flushes.total",
        /// Flushes caused by branch mispredict.
        flushes_branch: "pipeline.flushes.branch",
        /// Flushes caused by system-instruction serialization.
        flushes_system: "pipeline.flushes.system",
        /// Flushes caused by memory-ordering violations.
        flushes_mem_violations: "pipeline.flushes.mem_violations",
        /// Flushes for a value another hart overwrote.
        flushes_coherence: "pipeline.flushes.coherence",
        /// Flushes for an exception or interrupt taken at commit.
        flushes_trap: "pipeline.flushes.trap",
        /// Instructions squashed by flush events (misprediction penalty).
        flushes_squashed_insns: "pipeline.flushes.squashed_insns",
    }
}

stat_paths! {
    /// Branch-prediction counters.
    BpPaths {
        /// Committed branches whose prediction was correct.
        committed_hits: "bp.committed.hits",
        /// Committed branches whose prediction was wrong.
        committed_mispredicts: "bp.committed.mispredicts",
        /// Speculative branches whose prediction was correct (before commit).
        spec_hits: "bp.spec.hits",
        /// Speculative branches whose prediction was wrong.
        spec_mispredicts: "bp.spec.mispredicts",
        /// Times decode redirected fetch: a control instruction the BTB
        /// missed, a stale BTB target, or a BTB entry for a non-branch.
        decode_redirects: "bp.decode_redirects",
        /// Derived: committed prediction accuracy (hits / (hits+mispredicts)).
        committed_accuracy: "bp.committed.accuracy",
        /// Derived: speculative prediction accuracy.
        spec_accuracy: "bp.spec.accuracy",
    }
}

stat_paths! {
    /// Memory-dependence predictor counters.
    MdpPaths {
        /// Loads predicted to bypass all older stores.
        predictions_bypass: "mdp.predictions.bypass",
        /// Loads predicted to wait for all older stores.
        predictions_wait_all: "mdp.predictions.wait_all",
        /// Loads predicted to wait for a specific older store.
        predictions_wait_for: "mdp.predictions.wait_for",
        /// Load-after-store ordering violations observed at commit.
        violations: "mdp.violations",
    }
}

stat_paths! {
    /// Load/store unit counters.
    LsqPaths {
        /// Memory ops memory1 sent back for replay: loads partially
        /// overlapping an older store still in the store buffer, and LR/AMO
        /// ops behind an older store to the same address.
        rescheduled_mem_ops: "lsq.rescheduled_mem_ops",
        /// Stores whose data issued after their address.
        split_stores: "lsq.split_stores",
        /// LR / AMO instructions re-executed at commit because another
        /// hart wrote their line after they read it.
        coherence_replays: "lsq.coherence_replays",
        /// Younger loads squashed because another hart wrote their line
        /// before an older load to it read the new value.
        coherence_violations: "lsq.coherence_violations",
    }
}

stat_paths! {
    /// Write-combining buffer counters.
    WcbPaths {
        /// Store operations coalesced into an existing WCB line.
        coalesces: "wcb.coalesces",
        /// WCB lines drained to the memory hierarchy.
        drains: "wcb.drains",
    }
}

stat_paths! {
    /// The load/store unit's load prefetcher.
    LoadPrefetchPaths {
        /// Prefetches sent to fill the L1D.
        l1: "prefetch.loads.l1",
        /// Prefetches sent to fill the L2 alone, further ahead.
        l2: "prefetch.loads.l2",
        /// Prefetches not sent: past the trained page under `PageBoundary::Stop`.
        page_boundary: "prefetch.loads.dropped.page_boundary",
        /// Prefetches not sent: the next page's translation missed the DTLB.
        tlb_miss: "prefetch.loads.dropped.tlb_miss",
        /// Prefetches not sent: a page or region the load may not read.
        denied: "prefetch.loads.dropped.denied",
        /// Prefetches not sent: the line is not RAM.
        not_ram: "prefetch.loads.dropped.not_ram",
    }
}

/// Functional-unit names in `FuType` discriminant order; consumers index
/// [`FuPaths::all`] with the raw `FuType as usize`, so this order must match
/// the enum layout in `fu_pool.rs`.
const FU_NAMES: [&str; 17] = [
    "int_alu",
    "int_mul",
    "int_div",
    "fp_add",
    "fp_mul",
    "fp_fma",
    "fp_div_sqrt",
    "branch",
    "mem",
    "vec_int_alu",
    "vec_int_mul",
    "vec_int_div",
    "vec_fp_alu",
    "vec_fp_fma",
    "vec_fp_div_sqrt",
    "vec_mem",
    "vec_permute",
];

/// Functional-unit utilization: cycles each FU was busy.
#[derive(Clone, Copy, Debug)]
pub struct FuPaths {
    /// One path per FU type, in `FuType` discriminant order.
    pub all: [StatId; FU_NAMES.len()],
}

impl FuPaths {
    fn under(subject: &str) -> Self {
        let all =
            std::array::from_fn(|i| StatId::of(&format!("{subject}.fu.util.{}", FU_NAMES[i])));
        Self { all }
    }
}

/// Every path a physical core's pipeline writes (BP, MDP, LSQ, WCB, load
/// prefetcher and units included), rooted at `core<N>`; its caches keep
/// their own paths.
#[derive(Clone, Copy, Debug)]
pub struct CorePaths {
    /// `core<N>.commit.*`
    pub commit: CommitPaths,
    /// `core<N>.pipeline.*`
    pub pipeline: PipelinePaths,
    /// `core<N>.bp.*`
    pub bp: BpPaths,
    /// `core<N>.mdp.*`
    pub mdp: MdpPaths,
    /// `core<N>.lsq.*`
    pub lsq: LsqPaths,
    /// `core<N>.wcb.*`
    pub wcb: WcbPaths,
    /// `core<N>.prefetch.loads.*`
    pub load_prefetch: LoadPrefetchPaths,
    /// `core<N>.fu.util.*`
    pub fu: FuPaths,
    /// Derived: instructions per cycle.
    pub ipc: StatId,
    /// Derived: cycles per instruction.
    pub cpi: StatId,
}

impl CorePaths {
    /// Paths under `core<N>`.
    #[must_use]
    pub fn new(core: CoreId) -> Self {
        let subject = format!("core{}", core.val());
        Self {
            commit: CommitPaths::under(&subject),
            pipeline: PipelinePaths::under(&subject),
            bp: BpPaths::under(&subject),
            mdp: MdpPaths::under(&subject),
            lsq: LsqPaths::under(&subject),
            wcb: WcbPaths::under(&subject),
            load_prefetch: LoadPrefetchPaths::under(&subject),
            fu: FuPaths::under(&subject),
            ipc: StatId::of(&format!("{subject}.ipc")),
            cpi: StatId::of(&format!("{subject}.cpi")),
        }
    }
}

/// System-wide aggregates over every hart, rooted at `system`.
#[derive(Clone, Copy, Debug)]
pub struct SystemPaths {
    /// Instructions retired by all harts.
    pub retired_insts: StatId,
    /// Traps taken by all harts.
    pub traps: StatId,
}

impl SystemPaths {
    /// The single `system` subject.
    #[must_use]
    pub fn new() -> Self {
        Self {
            retired_insts: StatId::of("system.retired_insts"),
            traps: StatId::of("system.traps"),
        }
    }
}

impl Default for SystemPaths {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_carry_their_subject_index() {
        let core = CorePaths::new(CoreId::new(3));
        assert_eq!(core.commit.op_load.path(), "core3.commit.op.load");
        assert_eq!(core.fu.all[0].path(), "core3.fu.util.int_alu");
        assert_eq!(core.fu.all[16].path(), "core3.fu.util.vec_permute");
        assert_eq!(core.ipc.path(), "core3.ipc");
        let hart = HartPaths::new(HartId::new(7));
        assert_eq!(hart.cycles_kernel.path(), "hart7.cycles.kernel");
    }
}

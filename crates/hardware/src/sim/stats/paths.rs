//! Const path declarations for every stat the simulator writes.
//!
//! Using these constants at writer sites (instead of string literals) turns
//! typos into compile errors and makes rename-refactors trivial.
//!
//! Path grammar: `<subject>.<subsystem>[.<sub>].<counter>`. See
//! `docs/architecture/stats.md` for rationale.
//!
//! Single-hart configuration uses `core0.*` / `hart0.*` literal paths. When
//! multi-core lands, `{n}` interpolation will replace the fixed indices.

/// Architectural, per-hart counters (retired insts, traps, mode-cycle mix).
pub mod hart {
    /// Instructions retired (mirrors `SimState::instructions_retired`).
    pub const RETIRED_INSTS: &str = "hart0.retired_insts";
    /// Trap-taken events.
    pub const TRAPS: &str = "hart0.traps";
    /// Cycles spent in user (U) privilege.
    pub const CYCLES_USER: &str = "hart0.cycles.user";
    /// Cycles spent in supervisor (S) privilege.
    pub const CYCLES_KERNEL: &str = "hart0.cycles.kernel";
    /// Cycles spent in machine (M) privilege.
    pub const CYCLES_MACHINE: &str = "hart0.cycles.machine";
}

/// Physical execution-core (pipeline + private caches + BP + MDP + WCB).
pub mod core {
    /// Commit-stage counters — instruction-mix breakdown at retirement.
    pub mod commit {
        /// Integer load retired.
        pub const OP_LOAD: &str = "core0.commit.op.load";
        /// Integer store retired.
        pub const OP_STORE: &str = "core0.commit.op.store";
        /// Branch/jump retired.
        pub const OP_BRANCH: &str = "core0.commit.op.branch";
        /// Integer ALU retired.
        pub const OP_ALU: &str = "core0.commit.op.alu";
        /// System / CSR / ECALL retired.
        pub const OP_SYSTEM: &str = "core0.commit.op.system";

        /// FP load retired.
        pub const FP_LOAD: &str = "core0.commit.fp.load";
        /// FP store retired.
        pub const FP_STORE: &str = "core0.commit.fp.store";
        /// FP arithmetic retired.
        pub const FP_ARITH: &str = "core0.commit.fp.arith";
        /// FP fused multiply-add retired.
        pub const FP_FMA: &str = "core0.commit.fp.fma";
        /// FP divide/sqrt retired.
        pub const FP_DIV_SQRT: &str = "core0.commit.fp.div_sqrt";

        /// Vector integer op retired.
        pub const VEC_INT: &str = "core0.commit.vec.int";
        /// Vector FP op retired.
        pub const VEC_FP: &str = "core0.commit.vec.fp";
        /// Vector load retired.
        pub const VEC_LOAD: &str = "core0.commit.vec.load";
        /// Vector store retired.
        pub const VEC_STORE: &str = "core0.commit.vec.store";
        /// Vector misc (permute/mask/config) retired.
        pub const VEC_MISC: &str = "core0.commit.vec.misc";

        /// Retire histogram: cycles where 0 insts retired.
        pub const RETIRE_HIST_ZERO: &str = "core0.commit.retire_histogram.zero";
        /// Retire histogram: cycles where exactly 1 inst retired.
        pub const RETIRE_HIST_ONE: &str = "core0.commit.retire_histogram.one";
        /// Retire histogram: cycles where exactly 2 insts retired.
        pub const RETIRE_HIST_TWO: &str = "core0.commit.retire_histogram.two";
        /// Retire histogram: cycles where 3 or more insts retired.
        pub const RETIRE_HIST_THREE_PLUS: &str = "core0.commit.retire_histogram.three_plus";
    }

    /// Cross-stage pipeline counters (cycle categories, stalls, flushes).
    pub mod pipeline {
        /// Cycles the core spent in WFI (waiting for interrupt).
        pub const CYCLES_WFI: &str = "core0.pipeline.cycles.wfi";
        /// Cycles the ROB was empty.
        pub const CYCLES_ROB_EMPTY: &str = "core0.pipeline.cycles.rob_empty";

        /// Fetch stalled on control (front-end redirect pending).
        pub const STALLS_CONTROL: &str = "core0.pipeline.stalls.control";
        /// Fetch1 idle because an earlier fetch group is still waiting on
        /// the I-cache response or an instruction-fetch page walk.
        pub const STALLS_FETCH_WAIT: &str = "core0.pipeline.stalls.fetch_wait";
        /// Issue stalled on data hazard (source not ready).
        pub const STALLS_DATA: &str = "core0.pipeline.stalls.data";
        /// Issue stalled on FU structural hazard.
        pub const STALLS_FU_STRUCTURAL: &str = "core0.pipeline.stalls.fu_structural";
        /// Downstream backpressure stalls.
        pub const STALLS_BACKPRESSURE: &str = "core0.pipeline.stalls.backpressure";
        /// Dispatch stall (rename → issue queue).
        pub const STALLS_DISPATCH: &str = "core0.pipeline.stalls.dispatch";
        /// Checkpoint allocation stalls.
        pub const STALLS_CHECKPOINT: &str = "core0.pipeline.stalls.checkpoint";
        /// Squash-recovery cycles.
        pub const STALLS_SQUASH: &str = "core0.pipeline.stalls.squash";
        /// Rename-map rebuild cycles.
        pub const STALLS_RENAME_REBUILD: &str = "core0.pipeline.stalls.rename_rebuild";

        /// Total pipeline flushes.
        pub const FLUSHES_TOTAL: &str = "core0.pipeline.flushes.total";
        /// Flushes caused by branch mispredict.
        pub const FLUSHES_BRANCH: &str = "core0.pipeline.flushes.branch";
        /// Flushes caused by system-instruction serialization.
        pub const FLUSHES_SYSTEM: &str = "core0.pipeline.flushes.system";
        /// Flushes caused by memory-ordering violations.
        pub const FLUSHES_MEM_VIOLATIONS: &str = "core0.pipeline.flushes.mem_violations";
        /// Instructions squashed by flush events (misprediction penalty).
        pub const FLUSHES_SQUASHED_INSNS: &str = "core0.pipeline.flushes.squashed_insns";
    }

    /// Branch-prediction counters.
    pub mod bp {
        /// Committed branches whose prediction was correct.
        pub const COMMITTED_HITS: &str = "core0.bp.committed.hits";
        /// Committed branches whose prediction was wrong.
        pub const COMMITTED_MISPREDICTS: &str = "core0.bp.committed.mispredicts";
        /// Speculative branches whose prediction was correct (before commit).
        pub const SPEC_HITS: &str = "core0.bp.spec.hits";
        /// Speculative branches whose prediction was wrong.
        pub const SPEC_MISPREDICTS: &str = "core0.bp.spec.mispredicts";

        /// Derived: committed prediction accuracy (hits / (hits+mispredicts)).
        pub const COMMITTED_ACCURACY: &str = "core0.bp.committed.accuracy";
        /// Derived: speculative prediction accuracy.
        pub const SPEC_ACCURACY: &str = "core0.bp.spec.accuracy";
    }

    /// Memory-dependence predictor counters.
    pub mod mdp {
        /// Loads predicted to bypass all older stores.
        pub const PREDICTIONS_BYPASS: &str = "core0.mdp.predictions.bypass";
        /// Loads predicted to wait for all older stores.
        pub const PREDICTIONS_WAIT_ALL: &str = "core0.mdp.predictions.wait_all";
        /// Loads predicted to wait for a specific older store.
        pub const PREDICTIONS_WAIT_FOR: &str = "core0.mdp.predictions.wait_for";
        /// Load-after-store ordering violations observed at commit.
        pub const VIOLATIONS: &str = "core0.mdp.violations";
    }

    /// Load/store unit counters.
    pub mod lsq {
        /// Memory ops memory1 sent back for replay: loads partially
        /// overlapping an older store still in the store buffer, and LR/AMO
        /// ops behind an older store to the same address.
        pub const RESCHEDULED_MEM_OPS: &str = "core0.lsq.rescheduled_mem_ops";
        /// LR / AMO instructions re-executed at commit because another
        /// hart wrote their line after they read it.
        pub const COHERENCE_REPLAYS: &str = "core0.lsq.coherence_replays";
        /// Younger loads squashed because another hart wrote their line
        /// before an older load to it read the new value.
        pub const COHERENCE_VIOLATIONS: &str = "core0.lsq.coherence_violations";
    }

    /// Write-combining buffer counters.
    pub mod wcb {
        /// Store operations coalesced into an existing WCB line.
        pub const COALESCES: &str = "core0.wcb.coalesces";
        /// WCB lines drained to the memory hierarchy.
        pub const DRAINS: &str = "core0.wcb.drains";
    }

    /// Private cache counters.
    pub mod cache {
        /// L1D exclusive-line swaps into L2 (Phase 3c will expand this).
        pub const L1D_EXCLUSIVE_SWAPS: &str = "core0.cache.l1d.exclusive_swaps";
    }

    /// Functional-unit utilization: cycles each FU was busy.
    ///
    /// Indexed by the numeric `FuType` discriminant to match the writer's
    /// `fu_utilization[i]` array.
    pub mod fu {
        /// Integer ALU.
        pub const UTIL_INT_ALU: &str = "core0.fu.util.int_alu";
        /// Integer multiplier.
        pub const UTIL_INT_MUL: &str = "core0.fu.util.int_mul";
        /// Integer divider.
        pub const UTIL_INT_DIV: &str = "core0.fu.util.int_div";
        /// FP adder.
        pub const UTIL_FP_ADD: &str = "core0.fu.util.fp_add";
        /// FP multiplier.
        pub const UTIL_FP_MUL: &str = "core0.fu.util.fp_mul";
        /// FP fused multiply-add.
        pub const UTIL_FP_FMA: &str = "core0.fu.util.fp_fma";
        /// FP divide/sqrt.
        pub const UTIL_FP_DIV_SQRT: &str = "core0.fu.util.fp_div_sqrt";
        /// Branch / jump.
        pub const UTIL_BRANCH: &str = "core0.fu.util.branch";
        /// Memory (address gen for loads/stores).
        pub const UTIL_MEM: &str = "core0.fu.util.mem";
        /// Vector integer ALU.
        pub const UTIL_VEC_INT_ALU: &str = "core0.fu.util.vec_int_alu";
        /// Vector integer multiplier.
        pub const UTIL_VEC_INT_MUL: &str = "core0.fu.util.vec_int_mul";
        /// Vector integer divider.
        pub const UTIL_VEC_INT_DIV: &str = "core0.fu.util.vec_int_div";
        /// Vector FP ALU.
        pub const UTIL_VEC_FP_ALU: &str = "core0.fu.util.vec_fp_alu";
        /// Vector FP FMA.
        pub const UTIL_VEC_FP_FMA: &str = "core0.fu.util.vec_fp_fma";
        /// Vector FP divide/sqrt.
        pub const UTIL_VEC_FP_DIV_SQRT: &str = "core0.fu.util.vec_fp_div_sqrt";
        /// Vector memory.
        pub const UTIL_VEC_MEM: &str = "core0.fu.util.vec_mem";
        /// Vector permute / mask / config.
        pub const UTIL_VEC_PERMUTE: &str = "core0.fu.util.vec_permute";

        /// FU-utilization paths in FuType-discriminant order.
        ///
        /// Consumers index this array with the raw `FuType as usize` — the
        /// order MUST match the enum layout in `fu_pool.rs`.
        pub const ALL: [&str; 17] = [
            UTIL_INT_ALU,
            UTIL_INT_MUL,
            UTIL_INT_DIV,
            UTIL_FP_ADD,
            UTIL_FP_MUL,
            UTIL_FP_FMA,
            UTIL_FP_DIV_SQRT,
            UTIL_BRANCH,
            UTIL_MEM,
            UTIL_VEC_INT_ALU,
            UTIL_VEC_INT_MUL,
            UTIL_VEC_INT_DIV,
            UTIL_VEC_FP_ALU,
            UTIL_VEC_FP_FMA,
            UTIL_VEC_FP_DIV_SQRT,
            UTIL_VEC_MEM,
            UTIL_VEC_PERMUTE,
        ];
    }

    /// Derived: instructions per cycle.
    pub const IPC: &str = "core0.ipc";
    /// Derived: cycles per instruction.
    pub const CPI: &str = "core0.cpi";
}

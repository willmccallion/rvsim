//! Statistics Python binding.
//!
//! Exposes simulation statistics to Python: getters for cycles, cache hits/misses,
//! branch accuracy, and instruction mix; `print` / `print_sections` for human-readable
//! output; `to_dict` for JSON-serializable export (multisim, scripting).

use pyo3::prelude::*;
use rvsim_core::sim::stats::Stats;

/// Internal statistics wrapper — not exposed to Python.
#[derive(Clone)]
pub struct PyStats {
    pub stats: Stats,
    pub cycles: u64,
    pub instructions_retired: u64,
}

impl PyStats {
    /// Construct from a stats snapshot, cycle count, and retired-instruction count.
    pub const fn new(stats: Stats, cycles: u64, instructions_retired: u64) -> Self {
        Self { stats, cycles, instructions_retired }
    }

    /// Print all stats (full dump).
    pub fn print(&self) {
        println!("{}", self.stats.summary(self.cycles, self.instructions_retired));
    }

    /// Print only the given sections.
    ///
    /// `Stats::summary` renders the full auto-generated section set; per-section
    /// filtering is not yet plumbed through the hierarchical summary, so this
    /// currently emits the same output as `print`.
    pub fn print_sections(&self, _sections: Vec<String>) {
        self.print();
    }

    fn hier(&self, path: &str) -> u64 {
        self.stats.get(path).unwrap_or(0.0) as u64
    }

    /// Export all stats as a Python dict (JSON-serializable).
    pub fn to_dict(&self, py: Python<'_>) -> pyo3::PyResult<pyo3::Py<pyo3::types::PyDict>> {
        let d = pyo3::types::PyDict::new(py);
        d.set_item("cycles", self.cycles)?;
        d.set_item("instructions_retired", self.instructions_retired)?;
        d.set_item("stalls_control", self.hier("core0.pipeline.stalls.control"))?;
        d.set_item("stalls_data", self.hier("core0.pipeline.stalls.data"))?;
        d.set_item("stalls_fu_structural", self.hier("core0.pipeline.stalls.fu_structural"))?;
        d.set_item("stalls_backpressure", self.hier("core0.pipeline.stalls.backpressure"))?;
        d.set_item("misprediction_penalty", self.hier("core0.pipeline.flushes.squashed_insns"))?;
        d.set_item("pipeline_flushes", self.hier("core0.pipeline.flushes.total"))?;
        d.set_item("flushes_branch", self.hier("core0.pipeline.flushes.branch"))?;
        d.set_item("flushes_system", self.hier("core0.pipeline.flushes.system"))?;
        d.set_item("mem_ordering_violations", self.hier("core0.pipeline.flushes.mem_violations"))?;
        d.set_item("stalls_dispatch", self.hier("core0.pipeline.stalls.dispatch"))?;
        d.set_item("stalls_checkpoint", self.hier("core0.pipeline.stalls.checkpoint"))?;
        d.set_item("stalls_squash", self.hier("core0.pipeline.stalls.squash"))?;
        d.set_item("stalls_rename_rebuild", self.hier("core0.pipeline.stalls.rename_rebuild"))?;

        d.set_item("cycles_user", self.hier("hart0.cycles.user"))?;
        d.set_item("cycles_kernel", self.hier("hart0.cycles.kernel"))?;
        d.set_item("cycles_machine", self.hier("hart0.cycles.machine"))?;
        d.set_item("traps_taken", self.hier("hart0.traps"))?;

        let committed_hits = self.hier("core0.bp.committed.hits");
        let committed_mis = self.hier("core0.bp.committed.mispredicts");
        let spec_hits = self.hier("core0.bp.spec.hits");
        let spec_mis = self.hier("core0.bp.spec.mispredicts");
        d.set_item("branch_predictions", committed_hits)?;
        d.set_item("branch_mispredictions", committed_mis)?;
        d.set_item("speculative_branch_predictions", spec_hits)?;
        d.set_item("speculative_branch_mispredictions", spec_mis)?;

        let total_bp = committed_hits + committed_mis;
        let bp_acc =
            if total_bp > 0 { 100.0 * (committed_hits as f64 / total_bp as f64) } else { 0.0 };
        d.set_item("branch_accuracy_pct", bp_acc)?;

        let spec_total = spec_hits + spec_mis;
        let spec_acc =
            if spec_total > 0 { 100.0 * (spec_hits as f64 / spec_total as f64) } else { 0.0 };
        d.set_item("speculative_branch_accuracy_pct", spec_acc)?;
        let ipc = if self.cycles > 0 {
            self.instructions_retired as f64 / self.cycles as f64
        } else {
            0.0
        };
        d.set_item("ipc", ipc)?;

        d.set_item("inst_load", self.hier("core0.commit.op.load"))?;
        d.set_item("inst_store", self.hier("core0.commit.op.store"))?;
        d.set_item("inst_branch", self.hier("core0.commit.op.branch"))?;
        d.set_item("inst_alu", self.hier("core0.commit.op.alu"))?;
        d.set_item("inst_system", self.hier("core0.commit.op.system"))?;
        d.set_item("inst_fp_load", self.hier("core0.commit.fp.load"))?;
        d.set_item("inst_fp_store", self.hier("core0.commit.fp.store"))?;
        d.set_item("inst_fp_arith", self.hier("core0.commit.fp.arith"))?;
        d.set_item("inst_fp_fma", self.hier("core0.commit.fp.fma"))?;
        d.set_item("inst_fp_div_sqrt", self.hier("core0.commit.fp.div_sqrt"))?;

        d.set_item("inst_vec_int", self.hier("core0.commit.vec.int"))?;
        d.set_item("inst_vec_fp", self.hier("core0.commit.vec.fp"))?;
        d.set_item("inst_vec_load", self.hier("core0.commit.vec.load"))?;
        d.set_item("inst_vec_store", self.hier("core0.commit.vec.store"))?;
        d.set_item("inst_vec_misc", self.hier("core0.commit.vec.misc"))?;

        d.set_item("mdp_predictions_bypass", self.hier("core0.mdp.predictions.bypass"))?;
        d.set_item("mdp_predictions_wait_all", self.hier("core0.mdp.predictions.wait_all"))?;
        d.set_item("mdp_predictions_wait_for", self.hier("core0.mdp.predictions.wait_for"))?;
        d.set_item("mdp_violations", self.hier("core0.mdp.violations"))?;

        Ok(d.into())
    }
}

impl From<(Stats, u64, u64)> for PyStats {
    fn from((stats, cycles, instructions_retired): (Stats, u64, u64)) -> Self {
        Self { stats, cycles, instructions_retired }
    }
}

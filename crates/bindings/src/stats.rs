//! Statistics Python binding.
//!
//! Exposes simulation statistics to Python: getters for cycles, cache hits/misses,
//! branch accuracy, and instruction mix; `print` / `print_sections` for human-readable
//! output; `to_dict` for JSON-serializable export (multisim, scripting).

use pyo3::prelude::*;
use rvsim_core::stats::SimStats;

/// Internal statistics wrapper — not exposed to Python.
#[derive(Clone)]
pub struct PyStats {
    pub inner: SimStats,
}

impl PyStats {
    /// Print all stats (full dump).
    pub fn print(&self) {
        self.inner.print();
    }

    /// Print only the given sections.
    pub fn print_sections(&self, sections: Vec<String>) {
        self.inner.print_sections(&sections);
    }

    /// Export all stats as a Python dict (JSON-serializable).
    pub fn to_dict(&self, py: Python<'_>) -> pyo3::PyResult<pyo3::Py<pyo3::types::PyDict>> {
        let d = pyo3::types::PyDict::new(py);
        let s = &self.inner;
        d.set_item("cycles", s.cycles)?;
        d.set_item("instructions_retired", s.instructions_retired)?;
        d.set_item("icache_hits", s.icache_hits)?;
        d.set_item("icache_misses", s.icache_misses)?;
        d.set_item("dcache_hits", s.dcache_hits)?;
        d.set_item("dcache_misses", s.dcache_misses)?;
        d.set_item("l2_hits", s.l2_hits)?;
        d.set_item("l2_misses", s.l2_misses)?;
        d.set_item("l3_hits", s.l3_hits)?;
        d.set_item("l3_misses", s.l3_misses)?;
        d.set_item("stalls_mem", s.stalls_mem)?;
        d.set_item("stalls_control", s.stalls_control)?;
        d.set_item("stalls_data", s.stalls_data)?;

        d.set_item("cycles_user", s.cycles_user)?;
        d.set_item("cycles_kernel", s.cycles_kernel)?;
        d.set_item("cycles_machine", s.cycles_machine)?;
        d.set_item("traps_taken", s.traps_taken)?;

        d.set_item("branch_predictions", s.branch_predictions)?;
        d.set_item("branch_mispredictions", s.branch_mispredictions)?;
        let total_bp = s.branch_predictions + s.branch_mispredictions;
        let bp_acc = if total_bp > 0 {
            100.0 * (s.branch_predictions as f64 / total_bp as f64)
        } else {
            0.0
        };
        d.set_item("branch_accuracy_pct", bp_acc)?;
        let ipc = if s.cycles > 0 {
            s.instructions_retired as f64 / s.cycles as f64
        } else {
            0.0
        };
        d.set_item("ipc", ipc)?;

        d.set_item("inst_load", s.inst_load)?;
        d.set_item("inst_store", s.inst_store)?;
        d.set_item("inst_branch", s.inst_branch)?;
        d.set_item("inst_alu", s.inst_alu)?;
        d.set_item("inst_system", s.inst_system)?;
        d.set_item("inst_fp_load", s.inst_fp_load)?;
        d.set_item("inst_fp_store", s.inst_fp_store)?;
        d.set_item("inst_fp_arith", s.inst_fp_arith)?;
        d.set_item("inst_fp_fma", s.inst_fp_fma)?;
        d.set_item("inst_fp_div_sqrt", s.inst_fp_div_sqrt)?;

        Ok(d.into())
    }
}

impl From<SimStats> for PyStats {
    fn from(inner: SimStats) -> Self {
        PyStats { inner }
    }
}

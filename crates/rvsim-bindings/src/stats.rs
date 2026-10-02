//! Statistics Python binding.
//!
//! Exposes the hierarchical stats tree as `rvsim.Stats`. Path lookups
//! (`stats["core0.commit.op.load"]`), wildcard queries (`stats.query("**.hits")`),
//! subject aggregation, and the auto-summary all live on one class — no
//! separate dict interface.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyIterator, PyList};
use rvsim_core::stats::Stats;
use rvsim_core::system::StatsEpoch;

/// Python-facing view over the hierarchical stats tree.
///
/// Constructed by `PySimulator::stats` as a snapshot at
/// the read time. Supports path lookups, wildcard queries, subject-grouped
/// aggregation, and rendering the auto-generated summary.
#[pyclass(name = "Stats", module = "rvsim._core")]
#[derive(Clone, Debug)]
pub struct PyStats {
    stats: Stats,
    cycles: u64,
    instructions_retired: u64,
    /// The stats reset these counts start from.
    epoch: StatsEpoch,
}

impl PyStats {
    /// Snapshot the tree along with the top-level counters `summary` needs,
    /// counted from the reset at `epoch`.
    pub const fn new(
        stats: Stats,
        cycles: u64,
        instructions_retired: u64,
        epoch: StatsEpoch,
    ) -> Self {
        Self { stats, cycles, instructions_retired, epoch }
    }

    /// The stats of the region between `earlier` and this snapshot.
    ///
    /// # Errors
    ///
    /// Raises `ValueError` when the stats were reset between the two, or
    /// `earlier` was taken after this one.
    pub fn since(&self, earlier: &Self) -> PyResult<Self> {
        if earlier.epoch != self.epoch {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "the stats were reset between these snapshots",
            ));
        }
        if earlier.cycles > self.cycles {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "the snapshot being subtracted was taken later",
            ));
        }
        Ok(Self {
            stats: self.stats.since(&earlier.stats),
            cycles: self.cycles - earlier.cycles,
            instructions_retired: self.instructions_retired - earlier.instructions_retired,
            epoch: self.epoch,
        })
    }
}

#[pymethods]
impl PyStats {
    /// Total cycles simulated.
    #[getter]
    const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Instructions committed across all harts.
    #[getter]
    const fn instructions_retired(&self) -> u64 {
        self.instructions_retired
    }

    /// `later - earlier`: the stats of the region between two snapshots.
    /// Counters are subtracted and derived stats (IPC, miss rates) are
    /// recomputed from the differences; histograms keep exact counts and
    /// means but lose their minimum and maximum.
    fn __sub__(&self, earlier: &Self) -> PyResult<Self> {
        self.since(earlier)
    }

    /// Instructions per cycle (0.0 if cycles == 0).
    #[getter]
    fn ipc(&self) -> f64 {
        if self.cycles == 0 { 0.0 } else { self.instructions_retired as f64 / self.cycles as f64 }
    }

    /// Cycles per instruction (0.0 if `instructions_retired == 0`).
    #[getter]
    fn cpi(&self) -> f64 {
        if self.instructions_retired == 0 {
            0.0
        } else {
            self.cycles as f64 / self.instructions_retired as f64
        }
    }

    /// Read one stat by full path. Returns `None` for unregistered paths.
    /// Works for both raw counters and derived stats.
    #[pyo3(signature = (path, default=None))]
    fn get(&self, path: &str, default: Option<f64>) -> Option<f64> {
        self.stats.get(path).or(default)
    }

    /// `stats[path]` — same as `.get`, but raises `KeyError` on miss.
    fn __getitem__(&self, path: &str) -> PyResult<f64> {
        self.stats.get(path).ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(path.to_string()))
    }

    /// True if the path is registered (raw counter or derived stat).
    fn __contains__(&self, path: &str) -> bool {
        self.stats.get(path).is_some()
    }

    /// Runs a wildcard query and returns a [`PyQueryResult`].
    ///
    /// Pattern grammar:
    /// - `*` matches any characters within a single segment (`core*` → `core0`).
    /// - `**` matches zero or more full segments (`**.misses` matches at any depth).
    fn query(&self, pattern: &str) -> PyQueryResult {
        let matches: Vec<(String, f64)> =
            self.stats.query(pattern).iter().map(|(p, v)| (p.to_string(), v)).collect();
        PyQueryResult { matches }
    }

    /// Render the auto-generated summary as text.
    ///
    /// If `sections` is `None`, every top-level subject is included. Passing
    /// a list (e.g. `["core0", "hart0"]`) restricts output to those subjects.
    /// An empty list emits only the header.
    #[pyo3(signature = (sections=None))]
    fn summary(&self, sections: Option<Vec<String>>) -> String {
        sections.map_or_else(
            || self.stats.summary(self.cycles, self.instructions_retired),
            |list| {
                let refs: Vec<&str> = list.iter().map(String::as_str).collect();
                self.stats.summary_sections(self.cycles, self.instructions_retired, &refs)
            },
        )
    }

    /// The set of top-level subjects with registered stats (e.g. `["core0",
    /// "hart0"]`). Useful for feeding [`Self::summary`].
    fn subjects(&self) -> Vec<String> {
        self.stats.subjects().into_iter().map(String::from).collect()
    }

    /// `str(stats)` prints the summary.
    fn __str__(&self) -> String {
        self.stats.summary(self.cycles, self.instructions_retired)
    }

    fn __repr__(&self) -> String {
        format!(
            "Stats(cycles={}, instructions_retired={}, ipc={:.4})",
            self.cycles,
            self.instructions_retired,
            self.ipc()
        )
    }
}

/// A wildcard-query result set: `(path, value)` pairs with aggregation
/// helpers.
#[pyclass(name = "QueryResult", module = "rvsim._core")]
#[derive(Clone, Debug)]
pub struct PyQueryResult {
    matches: Vec<(String, f64)>,
}

#[pymethods]
impl PyQueryResult {
    /// Sum of all matched values (0.0 for an empty result).
    fn sum(&self) -> f64 {
        self.matches.iter().map(|(_, v)| *v).sum()
    }

    /// Number of matched paths.
    const fn __len__(&self) -> usize {
        self.matches.len()
    }

    /// True when no path matched.
    const fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Iterate over `(path, value)` tuples.
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyIterator>> {
        PyList::new(py, &self.matches)?.try_iter()
    }

    /// Return `{first_segment: sum}` — folds every matched path onto its
    /// top-level subject.
    fn by_subject<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (path, value) in &self.matches {
            let subject = path.split_once('.').map_or(path.as_str(), |(s, _)| s);
            let entry: f64 = d.get_item(subject)?.map_or(0.0, |v| v.extract().unwrap_or(0.0));
            d.set_item(subject, entry + *value)?;
        }
        Ok(d)
    }

    /// Return every matched path as a list of strings.
    fn paths(&self) -> Vec<String> {
        self.matches.iter().map(|(p, _)| p.clone()).collect()
    }

    fn __repr__(&self) -> String {
        format!("QueryResult(len={}, sum={:.4})", self.matches.len(), self.sum())
    }
}

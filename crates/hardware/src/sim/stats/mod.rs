//! Hierarchical, path-addressed statistics.
//!
//! Every counter lives at a path like `core0.commit.insts` or
//! `memctrl0.ch0.bank3.row_hits`. Paths carry no class-name leakage — swapping
//! branch predictors or cache implementations doesn't renumber anything.
//!
//! ## What this module gives you
//!
//! - Path-addressed [`Counter`]s and [`Histogram`]s ([`Stats::counter`]).
//! - Per-stat [`Meta`] with unit + aggregation kind ([`Stats::register`]).
//! - Derived metrics with formulas ([`Stats::derive`]), evaluated on read.
//! - Wildcard queries ([`Stats::query`]) with `*` (one segment) and `**`
//!   (any depth).
//! - Auto-generated summary ([`Stats::summary`]) driven by metadata.
//!
//! See `docs/architecture/stats.md` for the design rationale.

use std::collections::BTreeMap;
use std::io::{self, Write};

pub mod meta;
pub mod paths;
pub mod query;
pub mod summary;

pub use meta::{Kind, Meta, Unit};
pub use query::QueryResult;

/// A scalar counter.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counter(u64);

impl Counter {
    /// Increments the counter by one.
    #[inline]
    pub const fn inc(&mut self) {
        self.0 = self.0.saturating_add(1);
    }

    /// Adds `n` to the counter.
    #[inline]
    pub const fn add(&mut self, n: u64) {
        self.0 = self.0.saturating_add(n);
    }

    /// Returns the current value.
    #[inline]
    pub const fn get(&self) -> u64 {
        self.0
    }

    /// Resets the counter to zero.
    #[inline]
    pub const fn reset(&mut self) {
        self.0 = 0;
    }
}

/// A power-of-two-bucketed histogram for latency / queue-depth distributions.
///
/// Bucket `i` counts samples in `[2^i, 2^(i+1))`. Bucket 0 counts zeroes too.
#[derive(Clone, Debug, Default)]
pub struct Histogram {
    buckets: Vec<u64>,
    count: u64,
    sum: u64,
    min: u64,
    max: u64,
}

impl Histogram {
    /// Records a sample.
    pub fn record(&mut self, sample: u64) {
        let bucket = if sample <= 1 {
            0
        } else {
            (64 - sample.leading_zeros() - 1) as usize
        };
        if self.buckets.len() <= bucket {
            self.buckets.resize(bucket + 1, 0);
        }
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        if self.count == 0 || sample < self.min {
            self.min = sample;
        }
        if sample > self.max {
            self.max = sample;
        }
        self.count = self.count.saturating_add(1);
        self.sum = self.sum.saturating_add(sample);
    }

    /// Returns the total number of recorded samples.
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Returns the sum of all samples.
    pub const fn sum(&self) -> u64 {
        self.sum
    }

    /// Returns the mean of recorded samples, or 0 if no samples were recorded.
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum as f64 / self.count as f64
        }
    }

    /// Returns the minimum recorded sample, or 0 if no samples were recorded.
    pub const fn min(&self) -> u64 {
        self.min
    }

    /// Returns the maximum recorded sample, or 0 if no samples were recorded.
    pub const fn max(&self) -> u64 {
        self.max
    }

    /// Clears the histogram for phase-based analysis.
    pub fn reset(&mut self) {
        self.buckets.clear();
        self.count = 0;
        self.sum = 0;
        self.min = 0;
        self.max = 0;
    }
}

/// A single node in the stats tree. Holds local counters, histograms, and
/// child sub-groups keyed by path segment.
#[derive(Clone, Debug, Default)]
pub struct StatGroup {
    counters: BTreeMap<&'static str, Counter>,
    histograms: BTreeMap<&'static str, Histogram>,
    children: BTreeMap<&'static str, Self>,
}

impl StatGroup {
    /// Returns a mutable reference to the counter at `path`, creating it on first access.
    pub fn counter(&mut self, path: &'static str) -> &mut Counter {
        let (head, rest) = split_path(path);
        if let Some(rest) = rest {
            self.children.entry(head).or_default().counter(rest)
        } else {
            self.counters.entry(head).or_default()
        }
    }

    /// Returns a mutable reference to the histogram at `path`, creating it on first access.
    pub fn histogram(&mut self, path: &'static str) -> &mut Histogram {
        let (head, rest) = split_path(path);
        if let Some(rest) = rest {
            self.children.entry(head).or_default().histogram(rest)
        } else {
            self.histograms.entry(head).or_default()
        }
    }

    /// Reads the counter at `path` without creating it. Returns `None` if any
    /// intermediate segment or the leaf itself has never been written.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Counter> {
        let (head, rest) = path.find('.').map_or((path, None), |i| (&path[..i], Some(&path[i + 1..])));
        if let Some(rest) = rest {
            self.children.get(head)?.get(rest)
        } else {
            self.counters.get(head)
        }
    }

    /// Visits every counter in the tree in pre-order, invoking `visit` with
    /// the fully-qualified dotted path and a reference to the counter.
    pub fn walk(&self, prefix: &str, visit: &mut dyn FnMut(&str, &Counter)) {
        for (name, c) in &self.counters {
            let path = if prefix.is_empty() {
                (*name).to_string()
            } else {
                format!("{prefix}.{name}")
            };
            visit(&path, c);
        }
        for (name, child) in &self.children {
            let next = if prefix.is_empty() {
                (*name).to_string()
            } else {
                format!("{prefix}.{name}")
            };
            child.walk(&next, visit);
        }
    }

    /// Resets all counters and histograms recursively (phase boundary).
    pub fn reset(&mut self) {
        for c in self.counters.values_mut() {
            c.reset();
        }
        for h in self.histograms.values_mut() {
            h.reset();
        }
        for child in self.children.values_mut() {
            child.reset();
        }
    }

    fn dump_text(&self, prefix: &str, out: &mut dyn Write) -> io::Result<()> {
        for (name, c) in &self.counters {
            writeln!(out, "{prefix}{name} {}", c.get())?;
        }
        for (name, h) in &self.histograms {
            writeln!(
                out,
                "{prefix}{name} count={} sum={} mean={:.4} min={} max={}",
                h.count(),
                h.sum(),
                h.mean(),
                h.min(),
                h.max()
            )?;
        }
        for (name, child) in &self.children {
            let next = if prefix.is_empty() {
                format!("{name}.")
            } else {
                format!("{prefix}{name}.")
            };
            child.dump_text(&next, out)?;
        }
        Ok(())
    }
}

#[inline]
fn split_path(path: &'static str) -> (&'static str, Option<&'static str>) {
    path.find('.')
        .map_or((path, None), |idx| (&path[..idx], Some(&path[idx + 1..])))
}

/// Output format for `Stats::dump`.
#[derive(Clone, Copy, Debug)]
pub enum StatFormat {
    /// Human-readable `path.to.metric value` lines.
    Text,
}

/// A formula for a derived stat.
///
/// Deliberately narrow — enough for IPC/CPI/accuracy/miss-rate. Divide-by-zero
/// evaluates to `0.0`, not `NaN`, so runs with zero events don't poison
/// downstream analysis.
#[derive(Clone, Debug)]
pub enum Formula {
    /// `numerator / denominator`. Both are stat paths (raw or derived).
    Div(&'static str, &'static str),
    /// `numerator / (numerator + other)` — the common "accuracy" or
    /// "hit rate" shape.
    Ratio {
        /// Path whose value goes in the numerator and one term of the denominator.
        numerator: &'static str,
        /// Path whose value is the second term of the denominator.
        other: &'static str,
    },
    /// Sum of many paths. `Sum(&[])` is `0.0`.
    Sum(&'static [&'static str]),
}

/// A derived stat registration: formula + metadata.
#[derive(Clone, Debug)]
pub(crate) struct Derived {
    formula: Formula,
    #[allow(dead_code)] // metadata is looked up via `Stats::meta`; kept for parity
    meta: Meta,
}

/// Top-level statistics tree.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Root of the hierarchical tree.
    pub root: StatGroup,
    /// Per-registered-path metadata.
    pub(crate) meta: BTreeMap<&'static str, Meta>,
    /// Registered derived stats.
    pub(crate) derived: BTreeMap<&'static str, Derived>,
}

impl Stats {
    /// Creates an empty stats tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stats tree with the simulator's canonical metadata and
    /// derived formulas pre-registered. Every writer path appears in
    /// [`Stats::query`] and [`Stats::summary`] output even before any counter
    /// has been incremented.
    #[must_use]
    pub fn with_default_registrations() -> Self {
        let mut s = Self::new();
        register_defaults(&mut s);
        s
    }

    /// Returns a mutable reference to the counter at `path`.
    ///
    /// `path` uses `.` as a separator: `"core0.cache.l1d.hits"`.
    pub fn counter(&mut self, path: &'static str) -> &mut Counter {
        self.root.counter(path)
    }

    /// Returns a mutable reference to the histogram at `path`.
    pub fn histogram(&mut self, path: &'static str) -> &mut Histogram {
        self.root.histogram(path)
    }

    /// Registers metadata for a counter path. Idempotent — a repeat
    /// registration overwrites the previous [`Meta`].
    ///
    /// Registering also allocates the underlying [`Counter`] so the path
    /// appears in queries even before any writer has incremented it.
    pub fn register(&mut self, path: &'static str, meta: Meta) {
        let _ = self.root.counter(path);
        let _ = self.meta.insert(path, meta);
    }

    /// Registers a derived stat with a formula. The value is computed on read
    /// via [`Stats::get`] and included in [`Stats::summary`].
    pub fn derive(&mut self, path: &'static str, formula: Formula, meta: Meta) {
        let _ = self.derived.insert(path, Derived { formula, meta });
        let _ = self.meta.insert(path, meta);
    }

    /// Reads a stat value by path. Works for both raw counters and derived
    /// stats. Returns `None` if the path isn't registered / hasn't been
    /// written and isn't a derived formula.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<f64> {
        if let Some(derived) = self.derived.get(path) {
            return Some(self.eval(&derived.formula));
        }
        self.root.get(path).map(|c| c.get() as f64)
    }

    /// Evaluates a wildcard query against every path in the tree (raw and
    /// derived).
    #[must_use]
    pub fn query(&self, pattern: &str) -> QueryResult {
        let mut matches = Vec::new();
        let mut visit = |path: String, value: f64| {
            if query::matches(pattern, &path) {
                matches.push((path, value));
            }
        };
        self.root.walk("", &mut |p, c| visit(p.to_string(), c.get() as f64));
        for (path, derived) in &self.derived {
            visit((*path).to_string(), self.eval(&derived.formula));
        }
        QueryResult { matches }
    }

    /// Returns an auto-generated multi-section summary of the stats tree,
    /// grouped by subject and formatted according to each stat's [`Meta`].
    #[must_use]
    pub fn summary(&self, cycles: u64, instructions_retired: u64) -> String {
        summary::format(self, cycles, instructions_retired)
    }

    /// Same as [`Stats::summary`], but restricts output to a whitelist of
    /// top-level subjects (e.g. `["core0", "hart0"]`). An empty slice emits
    /// the header only.
    #[must_use]
    pub fn summary_sections(
        &self,
        cycles: u64,
        instructions_retired: u64,
        sections: &[&str],
    ) -> String {
        summary::format_sections(self, cycles, instructions_retired, Some(sections))
    }

    /// The set of top-level subjects that currently have at least one
    /// registered stat.
    #[must_use]
    pub fn subjects(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self
            .meta
            .keys()
            .map(|p| p.split_once('.').map_or(*p, |(head, _)| head))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Resets every counter and histogram in the tree. Metadata and derived
    /// registrations are preserved so the tree shape survives phase resets.
    pub fn reset(&mut self) {
        self.root.reset();
    }

    /// Writes the stats tree to `out` in the requested format.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error from the underlying writer.
    pub fn dump(&self, format: StatFormat, out: &mut dyn Write) -> io::Result<()> {
        match format {
            StatFormat::Text => self.root.dump_text("", out),
        }
    }

    /// Evaluates a formula against the current tree state. Divide-by-zero
    /// returns `0.0`.
    pub(crate) fn eval(&self, formula: &Formula) -> f64 {
        match formula {
            Formula::Div(num, den) => {
                let n = self.get(num).unwrap_or(0.0);
                let d = self.get(den).unwrap_or(0.0);
                if d == 0.0 { 0.0 } else { n / d }
            }
            Formula::Ratio { numerator, other } => {
                let n = self.get(numerator).unwrap_or(0.0);
                let o = self.get(other).unwrap_or(0.0);
                let total = n + o;
                if total == 0.0 { 0.0 } else { n / total }
            }
            Formula::Sum(paths) => paths.iter().map(|p| self.get(p).unwrap_or(0.0)).sum(),
        }
    }
}

/// Registers every simulator-owned counter path with its [`Meta`], plus the
/// derived rates ([`paths::core::IPC`], [`paths::core::CPI`], BP accuracies).
///
/// Kept as a plain function (not a method) so callers can compose it with
/// their own registrations if needed.
fn register_defaults(s: &mut Stats) {
    use paths::core::{bp, cache, commit, fu, lsq, mdp, pipeline as pipe, wcb};
    use paths::hart as h;

    // hart<N>.*
    s.register(h::RETIRED_INSTS, Meta::events("instructions retired"));
    s.register(h::TRAPS, Meta::events("trap-taken events"));
    s.register(h::CYCLES_USER, Meta::cycles("cycles spent in user (U) privilege"));
    s.register(h::CYCLES_KERNEL, Meta::cycles("cycles spent in supervisor (S) privilege"));
    s.register(h::CYCLES_MACHINE, Meta::cycles("cycles spent in machine (M) privilege"));

    // core<N>.commit.op.*
    s.register(commit::OP_LOAD, Meta::events("integer load retired"));
    s.register(commit::OP_STORE, Meta::events("integer store retired"));
    s.register(commit::OP_BRANCH, Meta::events("branch/jump retired"));
    s.register(commit::OP_ALU, Meta::events("integer ALU retired"));
    s.register(commit::OP_SYSTEM, Meta::events("system / CSR / ECALL retired"));

    // core<N>.commit.fp.*
    s.register(commit::FP_LOAD, Meta::events("FP load retired"));
    s.register(commit::FP_STORE, Meta::events("FP store retired"));
    s.register(commit::FP_ARITH, Meta::events("FP arithmetic retired"));
    s.register(commit::FP_FMA, Meta::events("FP fused multiply-add retired"));
    s.register(commit::FP_DIV_SQRT, Meta::events("FP divide/sqrt retired"));

    // core<N>.commit.vec.*
    s.register(commit::VEC_INT, Meta::events("vector integer op retired"));
    s.register(commit::VEC_FP, Meta::events("vector FP op retired"));
    s.register(commit::VEC_LOAD, Meta::events("vector load retired"));
    s.register(commit::VEC_STORE, Meta::events("vector store retired"));
    s.register(commit::VEC_MISC, Meta::events("vector misc (permute/mask/config) retired"));

    // core<N>.commit.retire_histogram.*
    s.register(commit::RETIRE_HIST_ZERO, Meta::cycles("cycles where 0 insts retired"));
    s.register(commit::RETIRE_HIST_ONE, Meta::cycles("cycles where exactly 1 inst retired"));
    s.register(commit::RETIRE_HIST_TWO, Meta::cycles("cycles where exactly 2 insts retired"));
    s.register(commit::RETIRE_HIST_THREE_PLUS, Meta::cycles("cycles where 3+ insts retired"));

    // core<N>.pipeline.*
    s.register(pipe::CYCLES_WFI, Meta::cycles("cycles in WFI"));
    s.register(pipe::CYCLES_ROB_EMPTY, Meta::cycles("cycles with empty ROB"));
    s.register(pipe::STALLS_CONTROL, Meta::cycles("fetch stalled on control"));
    s.register(pipe::STALLS_FETCH_WAIT, Meta::cycles("Fetch waited on an in-flight fetch"));
    s.register(pipe::STALLS_DATA, Meta::cycles("issue stalled on data hazard"));
    s.register(pipe::STALLS_FU_STRUCTURAL, Meta::cycles("issue stalled on FU structural"));
    s.register(pipe::STALLS_BACKPRESSURE, Meta::cycles("downstream backpressure stalls"));
    s.register(pipe::STALLS_DISPATCH, Meta::cycles("dispatch stalls"));
    s.register(pipe::STALLS_CHECKPOINT, Meta::cycles("checkpoint allocation stalls"));
    s.register(pipe::STALLS_SQUASH, Meta::cycles("squash-recovery cycles"));
    s.register(pipe::STALLS_RENAME_REBUILD, Meta::cycles("rename-map rebuild cycles"));
    s.register(pipe::FLUSHES_TOTAL, Meta::events("total pipeline flushes"));
    s.register(pipe::FLUSHES_BRANCH, Meta::events("flushes: branch mispredict"));
    s.register(pipe::FLUSHES_SYSTEM, Meta::events("flushes: system serialization"));
    s.register(pipe::FLUSHES_MEM_VIOLATIONS, Meta::events("flushes: memory ordering violations"));
    s.register(pipe::FLUSHES_SQUASHED_INSNS, Meta::events("insts squashed by flushes"));

    // core<N>.bp.*
    s.register(bp::COMMITTED_HITS, Meta::events("branch predictions correct (committed)"));
    s.register(bp::COMMITTED_MISPREDICTS, Meta::events("branch predictions wrong (committed)"));
    s.register(bp::SPEC_HITS, Meta::events("branch predictions correct (speculative)"));
    s.register(bp::SPEC_MISPREDICTS, Meta::events("branch predictions wrong (speculative)"));

    // core<N>.mdp.*
    s.register(mdp::PREDICTIONS_BYPASS, Meta::events("MDP predicted bypass"));
    s.register(mdp::PREDICTIONS_WAIT_ALL, Meta::events("MDP predicted wait-for-all"));
    s.register(mdp::PREDICTIONS_WAIT_FOR, Meta::events("MDP predicted wait-for-specific"));
    s.register(mdp::VIOLATIONS, Meta::events("MDP violations observed at commit"));

    // core<N>.lsq.*
    s.register(lsq::RESCHEDULED_MEM_OPS, Meta::events("Memory ops replayed behind an older store"));

    // core<N>.wcb.*
    s.register(wcb::COALESCES, Meta::events("WCB store coalesces"));
    s.register(wcb::DRAINS, Meta::events("WCB line drains"));

    // core<N>.cache.*
    s.register(cache::L1D_EXCLUSIVE_SWAPS, Meta::events("L1D exclusive-line swaps to L2"));

    // core<N>.fu.util.* — 17 paths in FuType-discriminant order.
    for path in fu::ALL {
        s.register(path, Meta::cycles("cycles this FU was busy"));
    }

    // Derived: BP accuracies.
    s.derive(
        bp::COMMITTED_ACCURACY,
        Formula::Ratio { numerator: bp::COMMITTED_HITS, other: bp::COMMITTED_MISPREDICTS },
        Meta::ratio("branch-prediction accuracy (committed)"),
    );
    s.derive(
        bp::SPEC_ACCURACY,
        Formula::Ratio { numerator: bp::SPEC_HITS, other: bp::SPEC_MISPREDICTS },
        Meta::ratio("branch-prediction accuracy (speculative)"),
    );

    // Derived: IPC / CPI.
    //
    // These are computed against `hart0.retired_insts`, which the commit stage
    // mirrors from `SimState::instructions_retired`. Total cycles live on
    // `SimState::cycle`, not in the tree — the summary layer supplies it, so
    // IPC/CPI here are placeholders that resolve once the writer landed in
    // Commit D wires `hart0.retired_insts` and a `core0.pipeline.cycles.total`
    // counter. Until then, both evaluate to 0.0 by the divide-by-zero rule.
    s.derive(
        paths::core::IPC,
        Formula::Div(h::RETIRED_INSTS, "core0.pipeline.cycles.total"),
        Meta::ratio("instructions per cycle"),
    );
    s.derive(
        paths::core::CPI,
        Formula::Div("core0.pipeline.cycles.total", h::RETIRED_INSTS),
        Meta::ratio("cycles per instruction"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_path_resolves_and_increments() {
        let mut s = Stats::new();
        s.counter("system.cpu0.commit.insts").add(42);
        s.counter("system.cpu0.commit.insts").inc();
        assert_eq!(s.counter("system.cpu0.commit.insts").get(), 43);
    }

    #[test]
    fn histogram_records_samples() {
        let mut s = Stats::new();
        for v in [1u64, 2, 4, 8, 16, 16, 16] {
            s.histogram("system.memctrl.latency").record(v);
        }
        let h = s.histogram("system.memctrl.latency");
        assert_eq!(h.count(), 7);
        assert_eq!(h.min(), 1);
        assert_eq!(h.max(), 16);
    }

    #[test]
    fn reset_clears_everything() {
        let mut s = Stats::new();
        s.counter("a.b").add(10);
        s.histogram("a.b.h").record(5);
        s.reset();
        assert_eq!(s.counter("a.b").get(), 0);
        assert_eq!(s.histogram("a.b.h").count(), 0);
    }

    #[test]
    fn dump_text_emits_paths() {
        let mut s = Stats::new();
        s.counter("root.left").add(1);
        s.counter("root.right").add(2);
        let mut buf = Vec::new();
        s.dump(StatFormat::Text, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("root.left 1"));
        assert!(out.contains("root.right 2"));
    }
}

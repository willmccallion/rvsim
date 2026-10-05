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

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::sync::{LazyLock, RwLock};

pub mod meta;
pub mod paths;
pub mod query;
pub mod summary;

pub use meta::{Kind, Meta, Unit};
pub use query::QueryResult;

use crate::common::{CoreId, HartId};
use paths::{CorePaths, HartPaths, SystemPaths};

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

    /// What was counted after `earlier`, a snapshot of this counter.
    #[must_use]
    pub const fn since(self, earlier: Self) -> Self {
        Self(self.0.saturating_sub(earlier.0))
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
    /// The smallest and largest samples; `None` with no samples, or for a
    /// window between two snapshots, whose extremes cannot be recovered.
    extremes: Option<(u64, u64)>,
}

impl Histogram {
    /// Records a sample.
    pub fn record(&mut self, sample: u64) {
        let bucket = if sample <= 1 { 0 } else { (64 - sample.leading_zeros() - 1) as usize };
        if self.buckets.len() <= bucket {
            self.buckets.resize(bucket + 1, 0);
        }
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.extremes = Some(match self.extremes {
            Some((min, max)) => (min.min(sample), max.max(sample)),
            None => (sample, sample),
        });
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
        if self.count == 0 { 0.0 } else { self.sum as f64 / self.count as f64 }
    }

    /// The smallest recorded sample, when known.
    pub fn min(&self) -> Option<u64> {
        self.extremes.map(|(min, _)| min)
    }

    /// The largest recorded sample, when known.
    pub fn max(&self) -> Option<u64> {
        self.extremes.map(|(_, max)| max)
    }

    /// Clears the histogram for phase-based analysis.
    pub fn reset(&mut self) {
        self.buckets.clear();
        self.count = 0;
        self.sum = 0;
        self.extremes = None;
    }

    /// The samples recorded after `earlier`, a snapshot of this histogram:
    /// exact buckets, count, sum and mean, but unknown extremes.
    #[must_use]
    pub fn since(&self, earlier: &Self) -> Self {
        let buckets = self
            .buckets
            .iter()
            .enumerate()
            .map(|(i, &n)| n.saturating_sub(earlier.buckets.get(i).copied().unwrap_or(0)))
            .collect();
        Self {
            buckets,
            count: self.count.saturating_sub(earlier.count),
            sum: self.sum.saturating_sub(earlier.sum),
            extremes: None,
        }
    }
}

/// A stat path, interned for the whole process: its text is kept once and
/// its id indexes every [`Stats`]' storage directly, so a hot increment is
/// an array index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatId(u32);

/// Every interned path and the id it has.
#[derive(Default)]
struct Registry {
    ids: HashMap<&'static str, StatId>,
    paths: Vec<&'static str>,
}

static REGISTRY: LazyLock<RwLock<Registry>> = LazyLock::new(RwLock::default);

fn registry() -> std::sync::RwLockReadGuard<'static, Registry> {
    REGISTRY.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl StatId {
    /// The id of `path`, interning it on first use.
    #[must_use]
    pub fn of(path: &str) -> Self {
        if let Some(id) = Self::find(path) {
            return id;
        }
        let mut registry = REGISTRY.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&id) = registry.ids.get(path) {
            return id;
        }
        let id = Self(u32::try_from(registry.paths.len()).unwrap_or(u32::MAX));
        let text: &'static str = Box::leak(path.to_owned().into_boxed_str());
        registry.paths.push(text);
        let _ = registry.ids.insert(text, id);
        id
    }

    /// The id of `path` if it was ever interned.
    #[must_use]
    pub fn find(path: &str) -> Option<Self> {
        registry().ids.get(path).copied()
    }

    /// The path's text.
    #[must_use]
    pub fn path(self) -> &'static str {
        registry().paths[self.0 as usize]
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<&str> for StatId {
    fn from(path: &str) -> Self {
        Self::of(path)
    }
}

/// Something that names a stat for reading: an id, or a path's text.
pub trait StatKey {
    /// The stat's id, if the path was ever interned.
    fn stat_id(&self) -> Option<StatId>;
}

impl StatKey for StatId {
    fn stat_id(&self) -> Option<StatId> {
        Some(*self)
    }
}

impl StatKey for str {
    fn stat_id(&self) -> Option<StatId> {
        StatId::find(self)
    }
}

impl<T: StatKey + ?Sized> StatKey for &T {
    fn stat_id(&self) -> Option<StatId> {
        (**self).stat_id()
    }
}

impl StatKey for String {
    fn stat_id(&self) -> Option<StatId> {
        StatId::find(self)
    }
}

/// One kind of stat, indexed by [`StatId`]: which ids this tree holds, and
/// their values.
#[derive(Clone, Debug)]
struct Store<T> {
    values: Vec<T>,
    present: Vec<bool>,
}

impl<T> Default for Store<T> {
    fn default() -> Self {
        Self { values: Vec::new(), present: Vec::new() }
    }
}

impl<T: Default + Clone> Store<T> {
    /// The value of `id`, created on first use.
    #[inline]
    fn get_or_insert(&mut self, id: StatId) -> &mut T {
        let index = id.index();
        if index >= self.values.len() {
            self.values.resize(index + 1, T::default());
            self.present.resize(index + 1, false);
        }
        self.present[index] = true;
        &mut self.values[index]
    }
}

impl<T> Store<T> {
    fn get(&self, id: StatId) -> Option<&T> {
        let index = id.index();
        self.present.get(index).copied().unwrap_or(false).then(|| &self.values[index])
    }

    /// Every id this tree holds, in path order.
    fn ids(&self) -> Vec<StatId> {
        let registry = registry();
        let mut ids: Vec<StatId> = (0..self.values.len())
            .filter(|&index| self.present[index])
            .map(|index| StatId(index as u32))
            .collect();
        ids.sort_by_key(|id| registry.paths[id.index()]);
        ids
    }

    /// The same ids with each value mapped through `f`, given the value
    /// `earlier` holds for it, if any.
    fn map_since(&self, earlier: &Self, f: impl Fn(&T, Option<&T>) -> T) -> Self {
        let values = (0..self.values.len())
            .map(|index| f(&self.values[index], earlier.get(StatId(index as u32))))
            .collect();
        Self { values, present: self.present.clone() }
    }
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
    /// `numerator / denominator`. Both are stats (raw or derived).
    Div(StatId, StatId),
    /// `numerator / (numerator + other)` — the common "accuracy" or
    /// "hit rate" shape.
    Ratio {
        /// The stat in the numerator and one term of the denominator.
        numerator: StatId,
        /// The second term of the denominator.
        other: StatId,
    },
    /// Sum of many stats. `Sum(&[])` is `0.0`.
    Sum(Vec<StatId>),
}

/// Top-level statistics tree.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    counters: Store<Counter>,
    histograms: Store<Histogram>,
    /// Per-registered-stat metadata.
    pub(crate) meta: BTreeMap<StatId, Meta>,
    /// Registered derived stats and the formulas that compute them.
    pub(crate) derived: BTreeMap<StatId, Formula>,
}

/// A component whose stats are registered with the tree it writes to.
pub trait StatSource {
    /// Registers the component's counters and derived stats with `stats`.
    fn register(&self, stats: &mut Stats);
}

impl Stats {
    /// Creates an empty stats tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stats tree with every hart's and core's canonical metadata
    /// and derived formulas pre-registered, plus the `system.*` sums. Every
    /// writer path appears in [`Stats::query`] and [`Stats::summary`] output
    /// even before any counter has been incremented. `components` register
    /// their own stats, in order, between the cores' and the system sums.
    #[must_use]
    pub fn for_components(
        harts: &[HartPaths],
        cores: &[(CorePaths, HartId)],
        components: &[&dyn StatSource],
    ) -> Self {
        let mut s = Self::new();
        for hart in harts {
            register_hart(&mut s, hart);
        }
        for (core, first_hart) in cores {
            register_core(&mut s, core, &harts[first_hart.as_index()]);
        }
        for component in components {
            component.register(&mut s);
        }
        register_system(&mut s, harts);
        s
    }

    /// [`Stats::for_components`] for one core hosting one hart, with no
    /// caches.
    #[must_use]
    pub fn with_default_registrations() -> Self {
        let hart = HartId::new(0);
        Self::for_components(
            &[HartPaths::new(hart)],
            &[(CorePaths::new(CoreId::new(0)), hart)],
            &[],
        )
    }

    /// Returns a mutable reference to the counter at `path`.
    ///
    /// `path` uses `.` as a separator: `"core0.cache.l1d.hits"`.
    #[inline]
    pub fn counter(&mut self, stat: impl Into<StatId>) -> &mut Counter {
        self.counters.get_or_insert(stat.into())
    }

    /// Returns a mutable reference to the histogram at `path`.
    pub fn histogram(&mut self, stat: impl Into<StatId>) -> &mut Histogram {
        self.histograms.get_or_insert(stat.into())
    }

    /// Reads the histogram `stat` names without creating it.
    #[must_use]
    pub fn histogram_at(&self, stat: impl StatKey) -> Option<&Histogram> {
        self.histograms.get(stat.stat_id()?)
    }

    /// Registers metadata for a counter path. Idempotent — a repeat
    /// registration overwrites the previous [`Meta`].
    ///
    /// Registering also allocates the underlying [`Counter`] so the path
    /// appears in queries even before any writer has incremented it.
    pub fn register(&mut self, stat: impl Into<StatId>, meta: Meta) {
        let stat = stat.into();
        let _ = self.counters.get_or_insert(stat);
        let _ = self.meta.insert(stat, meta);
    }

    /// Registers a derived stat with a formula. The value is computed on read
    /// via [`Stats::get`] and included in [`Stats::summary`].
    pub fn derive(&mut self, stat: impl Into<StatId>, formula: Formula, meta: Meta) {
        let stat = stat.into();
        let _ = self.derived.insert(stat, formula);
        let _ = self.meta.insert(stat, meta);
    }

    /// Reads a stat value by path. Works for both raw counters and derived
    /// stats. Returns `None` if the path isn't registered / hasn't been
    /// written and isn't a derived formula.
    #[must_use]
    pub fn get(&self, stat: impl StatKey) -> Option<f64> {
        let stat = stat.stat_id()?;
        if let Some(formula) = self.derived.get(&stat) {
            return Some(self.eval(formula));
        }
        self.counters.get(stat).map(|c| c.get() as f64)
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
        for id in self.counters.ids() {
            let value = self.counters.get(id).map_or(0.0, |c| c.get() as f64);
            visit(id.path().to_string(), value);
        }
        for (id, formula) in &self.derived {
            visit(id.path().to_string(), self.eval(formula));
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
            .map(|id| id.path())
            .map(|p| p.split_once('.').map_or(p, |(head, _)| head))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The stats accumulated after `earlier`, a snapshot of this tree:
    /// counters and histograms are subtracted, and derived stats (ratios)
    /// are recomputed from the differences when read.
    #[must_use]
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            counters: self
                .counters
                .map_since(&earlier.counters, |c, e| c.since(e.copied().unwrap_or_default())),
            histograms: self
                .histograms
                .map_since(&earlier.histograms, |h, e| e.map_or_else(|| h.clone(), |e| h.since(e))),
            meta: self.meta.clone(),
            derived: self.derived.clone(),
        }
    }

    /// Resets every counter and histogram in the tree. Metadata and derived
    /// registrations are preserved so the tree shape survives phase resets.
    pub fn reset(&mut self) {
        for counter in &mut self.counters.values {
            counter.reset();
        }
        for histogram in &mut self.histograms.values {
            histogram.reset();
        }
    }

    /// Writes the stats tree to `out` in the requested format.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error from the underlying writer.
    pub fn dump(&self, format: StatFormat, out: &mut dyn Write) -> io::Result<()> {
        match format {
            StatFormat::Text => self.dump_text(out),
        }
    }

    fn dump_text(&self, out: &mut dyn Write) -> io::Result<()> {
        for id in self.counters.ids() {
            let value = self.counters.get(id).map_or(0, Counter::get);
            writeln!(out, "{} {value}", id.path())?;
        }
        for id in self.histograms.ids() {
            let (path, Some(h)) = (id.path(), self.histograms.get(id)) else { continue };
            let known = |v: Option<u64>| v.map_or_else(|| "n/a".to_string(), |v| v.to_string());
            writeln!(
                out,
                "{path} count={} sum={} mean={:.4} min={} max={}",
                h.count(),
                h.sum(),
                h.mean(),
                known(h.min()),
                known(h.max())
            )?;
        }
        Ok(())
    }

    /// Evaluates a formula against the current tree state. Divide-by-zero
    /// returns `0.0`.
    pub(crate) fn eval(&self, formula: &Formula) -> f64 {
        match formula {
            Formula::Div(num, den) => {
                let n = self.get(*num).unwrap_or(0.0);
                let d = self.get(*den).unwrap_or(0.0);
                if d == 0.0 { 0.0 } else { n / d }
            }
            Formula::Ratio { numerator, other } => {
                let n = self.get(*numerator).unwrap_or(0.0);
                let o = self.get(*other).unwrap_or(0.0);
                let total = n + o;
                if total == 0.0 { 0.0 } else { n / total }
            }
            Formula::Sum(stats) => stats.iter().map(|&stat| self.get(stat).unwrap_or(0.0)).sum(),
        }
    }
}

/// Registers a hart's counters.
fn register_hart(s: &mut Stats, h: &HartPaths) {
    s.register(h.retired_insts, Meta::events("instructions retired"));
    s.register(h.traps, Meta::events("trap-taken events"));
    s.register(h.cycles_user, Meta::cycles("cycles spent in user (U) privilege"));
    s.register(h.cycles_kernel, Meta::cycles("cycles spent in supervisor (S) privilege"));
    s.register(h.cycles_machine, Meta::cycles("cycles spent in machine (M) privilege"));
}

/// Registers a core's counters and its derived rates (BP accuracies,
/// IPC/CPI against `first_hart`'s retired instructions).
fn register_core(s: &mut Stats, c: &CorePaths, first_hart: &HartPaths) {
    let commit = &c.commit;
    s.register(commit.op_load, Meta::events("integer load retired"));
    s.register(commit.op_store, Meta::events("integer store retired"));
    s.register(commit.op_branch, Meta::events("branch/jump retired"));
    s.register(commit.op_alu, Meta::events("integer ALU retired"));
    s.register(commit.op_system, Meta::events("system / CSR / ECALL retired"));
    s.register(commit.op_atomic, Meta::events("LR, SC or AMO retired"));
    s.register(commit.fp_load, Meta::events("FP load retired"));
    s.register(commit.fp_store, Meta::events("FP store retired"));
    s.register(commit.fp_arith, Meta::events("FP arithmetic retired"));
    s.register(commit.fp_fma, Meta::events("FP fused multiply-add retired"));
    s.register(commit.fp_div_sqrt, Meta::events("FP divide/sqrt retired"));
    s.register(commit.vec_int, Meta::events("vector integer op retired"));
    s.register(commit.vec_fp, Meta::events("vector FP op retired"));
    s.register(commit.vec_load, Meta::events("vector load retired"));
    s.register(commit.vec_store, Meta::events("vector store retired"));
    s.register(commit.vec_misc, Meta::events("vector misc (permute/mask/config) retired"));
    s.register(commit.vec_crypto, Meta::events("vector crypto (Zvk*) retired"));
    s.register(commit.retire_hist_zero, Meta::cycles("cycles where 0 insts retired"));
    s.register(commit.retire_hist_one, Meta::cycles("cycles where exactly 1 inst retired"));
    s.register(commit.retire_hist_two, Meta::cycles("cycles where exactly 2 insts retired"));
    s.register(commit.retire_hist_three_plus, Meta::cycles("cycles where 3+ insts retired"));

    let pipe = &c.pipeline;
    s.register(pipe.cycles_total, Meta::cycles("cycles the core was ticked"));
    s.register(pipe.cycles_wfi, Meta::cycles("cycles in WFI"));
    s.register(pipe.cycles_rob_empty, Meta::cycles("cycles with empty ROB"));
    s.register(pipe.stalls_control, Meta::cycles("recovering from a backend redirect"));
    s.register(pipe.stalls_fetch_wait, Meta::cycles("Fetch waited on an in-flight fetch"));
    s.register(pipe.stalls_data, Meta::cycles("issue stalled on data hazard"));
    s.register(pipe.stalls_ordering, Meta::cycles("issue held for program order"));
    s.register(pipe.stalls_fu_structural, Meta::cycles("issue stalled on FU structural"));
    s.register(pipe.stalls_backpressure, Meta::cycles("downstream backpressure stalls"));
    s.register(pipe.stalls_dispatch, Meta::cycles("dispatch stalls"));
    s.register(pipe.stalls_checkpoint, Meta::cycles("checkpoint allocation stalls"));
    s.register(
        pipe.stalls_serialize,
        Meta::cycles("rename stalls behind a serializing instruction"),
    );
    s.register(pipe.stalls_squash, Meta::cycles("squash-recovery cycles"));
    s.register(pipe.flushes_total, Meta::events("total pipeline flushes"));
    s.register(pipe.flushes_branch, Meta::events("flushes: branch mispredict"));
    s.register(pipe.flushes_system, Meta::events("flushes: system serialization"));
    s.register(pipe.flushes_mem_violations, Meta::events("flushes: memory ordering violations"));
    s.register(pipe.flushes_coherence, Meta::events("flushes: a value another hart overwrote"));
    s.register(pipe.flushes_trap, Meta::events("flushes: exceptions and interrupts at commit"));
    s.register(pipe.flushes_squashed_insns, Meta::events("ROB entries dropped by flushes"));

    let bp = &c.bp;
    s.register(bp.committed_hits, Meta::events("branch and jump predictions correct (committed)"));
    s.register(
        bp.committed_mispredicts,
        Meta::events("branch and jump predictions wrong (committed)"),
    );
    s.register(bp.spec_hits, Meta::events("branch predictions correct (speculative)"));
    s.register(bp.spec_mispredicts, Meta::events("branch predictions wrong (speculative)"));
    s.register(bp.decode_redirects, Meta::events("fetch redirects from decode"));

    let mdp = &c.mdp;
    s.register(mdp.predictions_bypass, Meta::events("MDP predicted bypass"));
    s.register(mdp.predictions_wait_all, Meta::events("MDP predicted wait-for-all"));
    s.register(mdp.predictions_wait_for, Meta::events("MDP predicted wait-for-specific"));
    s.register(mdp.violations, Meta::events("MDP violations observed at commit"));

    let lsq = &c.lsq;
    s.register(lsq.rescheduled_mem_ops, Meta::events("Memory ops replayed behind an older store"));
    s.register(lsq.split_stores, Meta::events("Stores whose data issued after their address"));
    s.register(
        lsq.coherence_replays,
        Meta::events("LR/AMO re-executed after a remote write to their line"),
    );
    s.register(
        lsq.coherence_violations,
        Meta::events("Loads squashed for reading a line before a remote write an older load saw"),
    );

    s.register(c.wcb.coalesces, Meta::events("WCB store coalesces"));
    s.register(c.wcb.drains, Meta::events("WCB line drains"));
    let pf = &c.load_prefetch;
    s.register(pf.l1, Meta::events("load prefetches sent to fill the L1D"));
    s.register(pf.l2, Meta::events("load prefetches sent to fill the L2 alone"));
    s.register(pf.page_boundary, Meta::events("load prefetches stopped at the page boundary"));
    s.register(pf.tlb_miss, Meta::events("load prefetches whose next page missed the DTLB"));
    s.register(pf.denied, Meta::events("load prefetches to a page the load may not read"));
    s.register(pf.not_ram, Meta::events("load prefetches to a line outside RAM"));
    s.register(c.cache.l1d_exclusive_swaps, Meta::events("L1D exclusive-line swaps to L2"));
    for path in c.fu.all {
        s.register(path, Meta::cycles("cycles this FU was busy"));
    }

    s.derive(
        bp.committed_accuracy,
        Formula::Ratio { numerator: bp.committed_hits, other: bp.committed_mispredicts },
        Meta::ratio("branch-prediction accuracy (committed)"),
    );
    s.derive(
        bp.spec_accuracy,
        Formula::Ratio { numerator: bp.spec_hits, other: bp.spec_mispredicts },
        Meta::ratio("branch-prediction accuracy (speculative)"),
    );
    s.derive(
        c.ipc,
        Formula::Div(first_hart.retired_insts, pipe.cycles_total),
        Meta::ratio("instructions per cycle"),
    );
    s.derive(
        c.cpi,
        Formula::Div(pipe.cycles_total, first_hart.retired_insts),
        Meta::ratio("cycles per instruction"),
    );
}

/// Registers the `system.*` sums over every hart.
fn register_system(s: &mut Stats, harts: &[HartPaths]) {
    let system = SystemPaths::new();
    let retired: Vec<StatId> = harts.iter().map(|h| h.retired_insts).collect();
    let traps: Vec<StatId> = harts.iter().map(|h| h.traps).collect();
    s.derive(
        system.retired_insts,
        Formula::Sum(retired),
        Meta::events("instructions retired by all harts"),
    );
    s.derive(system.traps, Formula::Sum(traps), Meta::events("traps taken by all harts"));
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
        assert_eq!(h.min(), Some(1));
        assert_eq!(h.max(), Some(16));
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

    #[test]
    fn a_window_recomputes_ratios_from_the_counts_it_holds() {
        let mut s = Stats::new();
        s.derive(
            "core.ipc",
            Formula::Div("core.insts".into(), "core.cycles".into()),
            Meta::events("ipc"),
        );
        s.counter("core.insts").add(100);
        s.counter("core.cycles").add(400);
        let start = s.clone();
        s.counter("core.insts").add(300);
        s.counter("core.cycles").add(100);

        let window = s.since(&start);

        assert_eq!(window.get("core.insts"), Some(300.0));
        assert_eq!(window.get("core.ipc"), Some(3.0), "not 0.8 - 0.25");
    }

    #[test]
    fn a_window_keeps_exact_histogram_counts_but_not_extremes() {
        let mut s = Stats::new();
        s.histogram("mem.latency").record(10);
        let start = s.clone();
        s.histogram("mem.latency").record(20);
        s.histogram("mem.latency").record(40);

        let window = s.since(&start);
        let h = window.histogram_at("mem.latency").cloned().expect("recorded");

        assert_eq!((h.count(), h.sum(), h.mean()), (2, 60, 30.0));
        assert_eq!((h.min(), h.max()), (None, None));
    }
}

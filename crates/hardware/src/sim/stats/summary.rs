//! Auto-generated summary formatting driven by registered [`Meta`].
//!
//! Walks the tree once, groups counters by their top-level subject
//! (`core0`, `hart0`, ...), and emits an aligned, unit-annotated report.
//! Derived stats (IPC/CPI/accuracy) appear alongside the raw counters they
//! summarize.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::meta::Unit;
use super::{Meta, Stats};

/// Renders the stats tree as a human-readable summary string.
///
/// `cycles` and `instructions_retired` are passed in because they live on
/// [`SimState`](crate::sim::state::SimState), not in the tree itself. They're
/// emitted first as a fixed header so the derived rates that follow have
/// context.
pub fn format(stats: &Stats, cycles: u64, instructions_retired: u64) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "cycles                {cycles}");
    let _ = writeln!(out, "instructions_retired  {instructions_retired}");

    let entries = collect_entries(stats);
    if entries.is_empty() {
        return out;
    }

    let mut by_subject: BTreeMap<&str, Vec<Entry<'_>>> = BTreeMap::new();
    for entry in &entries {
        by_subject.entry(entry.subject).or_default().push(entry.clone());
    }

    let name_width = entries.iter().map(|e| e.tail.len()).max().unwrap_or(0).max(20);

    for (subject, mut rows) in by_subject {
        rows.sort_by(|a, b| a.tail.cmp(b.tail));
        let _ = writeln!(out);
        let _ = writeln!(out, "[{subject}]");
        for row in rows {
            let value_str = format_value(row.value, row.meta.unit);
            let _ = writeln!(out, "  {:<name_width$}  {value_str}", row.tail);
        }
    }

    out
}

#[derive(Clone)]
struct Entry<'a> {
    subject: &'a str,
    tail: &'a str,
    value: f64,
    meta: Meta,
}

fn collect_entries(stats: &Stats) -> Vec<Entry<'_>> {
    let mut entries = Vec::new();
    for (path, meta) in &stats.meta {
        let Some(value) = stats.get(path) else { continue };
        let (subject, tail) = path.split_once('.').unwrap_or((path, ""));
        entries.push(Entry { subject, tail, value, meta: *meta });
    }
    entries
}

fn format_value(value: f64, unit: Unit) -> String {
    match unit {
        Unit::Events | Unit::Cycles | Unit::Bytes => format!("{}", value as u64),
        Unit::Ratio => format!("{value:.4}"),
        Unit::Rate => format!("{value:.2}"),
        Unit::Percent => format!("{value:.2}%"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Formula, Meta};
    use super::*;

    #[test]
    fn empty_stats_emits_header_only() {
        let stats = Stats::new();
        let s = format(&stats, 0, 0);
        assert!(s.contains("cycles"));
        assert!(s.contains("instructions_retired"));
    }

    #[test]
    fn groups_by_subject_and_includes_derived() {
        let mut stats = Stats::new();
        stats.register("core0.commit.op.load", Meta::events("integer load retired"));
        stats.register("core0.commit.op.store", Meta::events("integer store retired"));
        stats.derive(
            "core0.ipc",
            Formula::Div("core0.commit.op.load", "core0.commit.op.store"),
            Meta::ratio("instructions per cycle"),
        );
        stats.counter("core0.commit.op.load").add(10);
        stats.counter("core0.commit.op.store").add(5);

        let out = format(&stats, 100, 42);
        assert!(out.contains("[core0]"));
        assert!(out.contains("commit.op.load"));
        assert!(out.contains("ipc"));
    }
}

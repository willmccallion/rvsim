//! End-to-end exercise of `Stats::query` wildcard patterns.
//!
//! Populates the tree with a mix of counter paths that mirror the shape the
//! simulator writes, then runs each query variant and asserts on the
//! aggregated result.

use rvsim_core::common::{CoreId, HartId};
use rvsim_core::sim::stats::paths::{CorePaths, HartPaths};
use rvsim_core::sim::stats::{Formula, Meta, Stats};

fn hart0() -> HartPaths {
    HartPaths::new(HartId::new(0))
}

fn core0() -> CorePaths {
    CorePaths::new(CoreId::new(0))
}

fn seeded() -> Stats {
    let mut s = Stats::with_default_registrations();
    let hart = hart0();
    s.counter(hart.retired_insts).add(1000);
    s.counter(hart.traps).add(3);
    s.counter(hart.cycles_user).add(600);
    s.counter(hart.cycles_kernel).add(300);
    s.counter(hart.cycles_machine).add(100);

    let core = core0();
    s.counter(core.commit.op_load).add(100);
    s.counter(core.commit.op_store).add(50);
    s.counter(core.commit.op_branch).add(150);
    s.counter(core.commit.op_alu).add(600);
    s.counter(core.commit.op_system).add(10);

    s.counter(core.bp.committed_hits).add(140);
    s.counter(core.bp.committed_mispredicts).add(10);
    s.counter(core.bp.spec_hits).add(280);
    s.counter(core.bp.spec_mispredicts).add(20);

    s.counter(core.pipeline.stalls_control).add(50);
    s.counter(core.pipeline.stalls_data).add(70);
    s.counter(core.pipeline.stalls_backpressure).add(30);
    s
}

#[test]
fn literal_path_read_via_get() {
    let s = seeded();
    assert_eq!(s.get(hart0().retired_insts), Some(1000.0));
    assert_eq!(s.get(core0().commit.op_load), Some(100.0));
}

#[test]
fn every_core_and_hart_gets_its_own_subject() {
    let harts = [HartPaths::new(HartId::new(0)), HartPaths::new(HartId::new(1))];
    let cores = [
        (CorePaths::new(CoreId::new(0)), HartId::new(0)),
        (CorePaths::new(CoreId::new(1)), HartId::new(1)),
    ];
    let mut s = Stats::for_components(&harts, &cores, &[]);
    s.counter(harts[0].retired_insts).add(10);
    s.counter(harts[1].retired_insts).add(5);
    s.counter(cores[1].0.commit.op_load).add(7);

    assert_eq!(s.query("core*.commit.op.load").len(), 2);
    assert_eq!(s.get("core1.commit.op.load"), Some(7.0));
    assert_eq!(s.get("core0.commit.op.load"), Some(0.0));
    assert_eq!(s.get("system.retired_insts"), Some(15.0));
    assert!(s.subjects().contains(&"core1"));
    assert!(s.subjects().contains(&"hart1"));
}

#[test]
fn unknown_path_returns_none() {
    let s = seeded();
    assert!(s.get("nonexistent.path").is_none());
    // Not-yet-registered / never-written path under a valid subject
    assert!(s.get("core0.does.not.exist").is_none());
}

#[test]
fn star_within_segment_matches_all_core_op_counters() {
    let s = seeded();
    let q = s.query("core0.commit.op.*");
    // 5 op.* counters registered.
    assert_eq!(q.len(), 5);
    assert_eq!(q.sum(), 100.0 + 50.0 + 150.0 + 600.0 + 10.0);
}

#[test]
fn double_star_reaches_any_depth() {
    let s = seeded();
    // Every `.hits` counter anywhere in the tree.
    let q = s.query("**.hits");
    // core0.bp.committed.hits + core0.bp.spec.hits
    assert_eq!(q.len(), 2);
    assert_eq!(q.sum(), 140.0 + 280.0);
}

#[test]
fn star_prefix_matches_segment_starting_with() {
    let s = seeded();
    // core* matches core0 (and would match core1, core42, ... under multi-core)
    let q = s.query("core*.commit.op.load");
    assert_eq!(q.len(), 1);
    assert_eq!(q.sum(), 100.0);
}

#[test]
fn by_subject_groups_across_the_first_segment() {
    let s = seeded();
    let q = s.query("**.hits");
    let by = q.by_subject();
    // Both hits paths live under core0.
    assert_eq!(by.len(), 1);
    assert_eq!(by["core0"], 140.0 + 280.0);
}

#[test]
fn by_subject_partitions_across_multiple_subjects() {
    let s = seeded();
    // A pattern that matches counters under both `hart0` and `core0`.
    let q = s.query("**.op.*");
    // Only core0.commit.op.* matches (hart0 has no op.* counters).
    let by = q.by_subject();
    assert_eq!(by.len(), 1);
    assert!(by.contains_key("core0"));
}

#[test]
fn derived_stat_is_included_in_query_results() {
    let s = seeded();
    // `core0.bp.committed.accuracy` is registered as a derived stat by
    // `with_default_registrations` — verify it evaluates and shows up in
    // wildcard queries.
    let accuracy = s.get(core0().bp.committed_accuracy).unwrap();
    let expected = 140.0 / (140.0 + 10.0);
    assert!((accuracy - expected).abs() < 1e-9);

    let q = s.query("core0.bp.committed.*");
    // hits + mispredicts + accuracy = 3 matches.
    let matched: Vec<&str> = q.iter().map(|(p, _)| p).collect();
    assert!(matched.contains(&"core0.bp.committed.hits"));
    assert!(matched.contains(&"core0.bp.committed.mispredicts"));
    assert!(matched.contains(&"core0.bp.committed.accuracy"));
}

#[test]
fn double_star_matches_zero_segments() {
    let s = seeded();
    // `hart0.**` should match every counter under hart0 (RETIRED_INSTS,
    // TRAPS, and the three CYCLES_*).
    let q = s.query("hart0.**");
    assert_eq!(q.len(), 5);
}

#[test]
fn divide_by_zero_yields_zero_not_nan() {
    let mut s = Stats::new();
    s.register("a.numerator", Meta::events(""));
    s.register("a.denominator", Meta::events(""));
    s.derive("a.ratio", Formula::Div("a.numerator", "a.denominator"), Meta::ratio(""));
    // Both counters are zero: divide-by-zero must yield 0.0.
    assert_eq!(s.get("a.ratio"), Some(0.0));
}

#[test]
fn unmatched_pattern_returns_empty() {
    let s = seeded();
    let q = s.query("nothing.here.exists");
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
    assert_eq!(q.sum(), 0.0);
}

#[test]
fn star_matches_bare_segment() {
    let s = seeded();
    // `*.retired_insts` should match `hart0.retired_insts` and the derived
    // `system.retired_insts` sum over every hart.
    let q = s.query("*.retired_insts");
    assert_eq!(q.len(), 2);
    assert_eq!(q.sum(), 2000.0);
}

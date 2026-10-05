//! Every stat the simulator registers is checked by an accounting check, so
//! a stat cannot be added, or keep counting, without one.

use std::collections::BTreeSet;

use super::{Recorder, caches, coherence, commit, ddr5, fu, pipeline, predictors};
use crate::Simulator;
use crate::config::{Config, MemoryControllerKind};

/// Every accounting check.
fn every_check() -> impl Iterator<Item = &'static fn(&mut Recorder)> {
    [
        commit::CHECKS,
        pipeline::CHECKS,
        predictors::CHECKS,
        fu::CHECKS,
        caches::CHECKS,
        coherence::CHECKS,
        ddr5::CHECKS,
    ]
    .into_iter()
    .flatten()
}

/// A system with every component that has stats: every cache level and a
/// DDR5 controller, on `harts` cores.
fn every_component(harts: usize) -> Config {
    let mut config = Config::default();
    config.system.hart_count = harts;
    config.system.console = crate::config::Console::Quiet;
    for cache in
        [&mut config.cache.l1_i, &mut config.cache.l1_d, &mut config.cache.l2, &mut config.cache.l3]
    {
        cache.enabled = true;
    }
    config.memory.controller = MemoryControllerKind::Ddr5;
    config
}

/// `path` with the index of every core, hart, controller, channel and
/// subchannel replaced by `*`, so `core1.commit.op.alu` and
/// `core0.commit.op.alu` are one stat.
fn normalized(path: &str) -> String {
    let segment = |part: &str| {
        for prefix in ["core", "hart", "memctrl", "ch", "sc"] {
            if let Some(index) = part.strip_prefix(prefix)
                && !index.is_empty()
                && index.bytes().all(|b| b.is_ascii_digit())
            {
                return format!("{prefix}*");
            }
        }
        part.to_owned()
    };
    path.split('.').map(segment).collect::<Vec<_>>().join(".")
}

/// Every stat registered by one core and by two coherent cores.
fn registered() -> BTreeSet<String> {
    [1, 2]
        .into_iter()
        .flat_map(|harts| {
            let sim = Simulator::build(&every_component(harts), "");
            sim.stats().meta.keys().map(|id| normalized(id.path())).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn every_registered_stat_is_checked_by_an_accounting_check() {
    let mut rec = Recorder::default();
    for check in every_check() {
        check(&mut rec);
    }

    let checked: BTreeSet<String> = rec.checked.iter().map(|path| normalized(path)).collect();
    let registered = registered();
    let unchecked: Vec<&String> = registered.difference(&checked).collect();

    assert!(
        unchecked.is_empty(),
        "{} registered stats no accounting check covers:\n{}",
        unchecked.len(),
        unchecked.iter().map(|path| path.as_str()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn indices_are_normalized_but_names_are_not() {
    assert_eq!(normalized("core12.cache.l1d.hits"), "core*.cache.l1d.hits");
    assert_eq!(normalized("memctrl0.ch1.sc0.row_hits"), "memctrl*.ch*.sc*.row_hits");
    assert_eq!(normalized("hart3.cycles.user"), "hart*.cycles.user");
    assert_eq!(normalized("coherence.ha.c2c_transfers"), "coherence.ha.c2c_transfers");
    assert_eq!(normalized("core0.pipeline.cycles.total"), "core*.pipeline.cycles.total");
}

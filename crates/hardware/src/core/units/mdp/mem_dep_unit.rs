//! Memory Dependence Unit — gem5-style dependency tracker with wakeup.
//!
//! Owns a predictor (Blind or StoreSet) and maintains a per-instruction
//! dependency map. The pipeline queries this unit at dispatch to get a
//! cached [`MemDepState`] for each instruction, and notifies it when
//! stores resolve so that waiting instructions can be woken.

use std::collections::HashMap;

use crate::config::{Config, MemDepPredictor as MdpType};
use crate::core::pipeline::rob::{Rob, RobTag};

use super::mem_dep_predictor::{MdpStats, MemDepPredictor, MemPrediction};
use super::store_set::StoreSetPredictor;

/// Cached memory dependency state for an IQ entry.
///
/// Set once at dispatch time by [`MemDepUnit::dispatch`]. The issue queue
/// checks this every cycle instead of re-querying the predictor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MemDepState {
    /// Not memory, or store with no chain predecessor.
    #[default]
    None,
    /// Load predicted independent (can bypass all older stores).
    Bypass,
    /// Must wait for all older stores to resolve.
    WaitAll,
    /// Must wait for a specific older instruction to resolve its address.
    WaitFor(RobTag),
    /// Was waiting on a specific store that has now resolved — can issue.
    Resolved(RobTag),
}

/// Memory Dependence Unit — owns a predictor and dependency tracking.
#[derive(Debug)]
pub struct MemDepUnit {
    predictor: PredictorKind,
    /// In-flight dependency records: `waiter_rob_tag.0` → barrier it waits on.
    deps: HashMap<u32, DepRecord>,
    /// Aggregated statistics (`bypass`/`wait_all`/`wait_for`/violations).
    stats: MdpStats,
}

/// Static dispatch for predictor implementations.
#[derive(Debug)]
enum PredictorKind {
    /// Conservative: loads always wait for all older stores.
    Blind,
    /// Store-set predictor: learns dependencies from violations.
    StoreSet(Box<StoreSetPredictor>),
}

/// Record of an in-flight memory dependency.
#[derive(Debug)]
struct DepRecord {
    /// The instruction this waiter depends on.
    barrier: RobTag,
    /// Whether the barrier has resolved its address.
    resolved: bool,
}

impl MemDepUnit {
    /// Creates a new `MemDepUnit` from configuration.
    pub fn new(config: &Config) -> Self {
        let predictor = match config.pipeline.mem_dep_predictor {
            MdpType::Blind => PredictorKind::Blind,
            MdpType::StoreSet => PredictorKind::StoreSet(Box::new(StoreSetPredictor::new(
                &config.pipeline.store_set,
            ))),
        };
        Self { predictor, deps: HashMap::new(), stats: MdpStats::default() }
    }

    /// Called at dispatch for every instruction.
    ///
    /// Returns the [`MemDepState`] to cache in the IQ entry.
    /// For stores, also registers them in the LFST (after querying for
    /// their chain predecessor).
    pub fn dispatch(
        &mut self,
        pc: u64,
        rob_tag: RobTag,
        is_load: bool,
        is_store: bool,
    ) -> MemDepState {
        match &mut self.predictor {
            PredictorKind::Blind => {
                if is_load {
                    self.stats.predictions_wait_all += 1;
                    MemDepState::WaitAll
                } else {
                    MemDepState::None
                }
            }
            PredictorKind::StoreSet(predictor) => {
                if !is_load && !is_store {
                    return MemDepState::None;
                }
                let prediction = predictor.predict(pc, rob_tag, is_store);
                if is_store {
                    predictor.register_store(pc, rob_tag);
                }
                match prediction {
                    MemPrediction::NoDep => {
                        if is_load {
                            self.stats.predictions_bypass += 1;
                            MemDepState::Bypass
                        } else {
                            MemDepState::None
                        }
                    }
                    MemPrediction::DepOn(barrier) => {
                        if barrier.is_older_than(rob_tag) {
                            let _ =
                                self.deps.insert(rob_tag.0, DepRecord { barrier, resolved: false });
                            self.stats.predictions_wait_for += 1;
                            MemDepState::WaitFor(barrier)
                        } else {
                            // Younger barrier = LFST alias, ignore.
                            if is_load {
                                self.stats.predictions_bypass += 1;
                                MemDepState::Bypass
                            } else {
                                MemDepState::None
                            }
                        }
                    }
                }
            }
        }
    }

    /// Called when a store resolves its address (in memory2).
    ///
    /// Marks all deps waiting on this store as resolved.
    /// Returns the store's `RobTag` so the IQ can wake entries whose
    /// `WaitFor(barrier)` matches it.
    pub fn store_resolved(&mut self, store_rob_tag: RobTag) -> Option<RobTag> {
        let mut any_woken = false;
        for dep in self.deps.values_mut() {
            if dep.barrier == store_rob_tag && !dep.resolved {
                dep.resolved = true;
                any_woken = true;
            }
        }
        if any_woken { Some(store_rob_tag) } else { None }
    }

    /// Called when an instruction is issued from the IQ.
    ///
    /// Removes its dependency record (cleanup).
    pub fn issued(&mut self, rob_tag: RobTag) {
        let _ = self.deps.remove(&rob_tag.0);
    }

    /// Train on violation detection.
    pub fn violation(&mut self, load_pc: u64, store_pc: u64) {
        self.stats.violations += 1;
        if let PredictorKind::StoreSet(predictor) = &mut self.predictor {
            predictor.train(load_pc, store_pc);
        }
    }

    /// Full flush — clear all dep records and LFST.
    pub fn flush(&mut self) {
        self.deps.clear();
        if let PredictorKind::StoreSet(predictor) = &mut self.predictor {
            predictor.flush();
        }
    }

    /// Partial flush with LFST rebuild from surviving ROB stores.
    ///
    /// Must be called AFTER `rob.flush_after(keep_tag)` so the ROB only
    /// contains surviving entries when we walk it for LFST rebuild.
    pub fn flush_after(&mut self, keep_tag: RobTag, rob: &Rob) {
        self.deps.retain(|&tag, _| RobTag(tag).is_older_or_eq(keep_tag));
        if let PredictorKind::StoreSet(predictor) = &mut self.predictor {
            predictor.flush_after(keep_tag);
            // Rebuild LFST from surviving stores in program order.
            for entry in rob.iter_in_order() {
                if entry.ctrl.mem_write {
                    predictor.rebuild_lfst_entry(entry.pc, entry.tag);
                }
            }
        }
    }

    /// Per-cycle tick.
    pub fn tick(&mut self) {
        if let PredictorKind::StoreSet(predictor) = &mut self.predictor {
            predictor.tick();
        }
    }

    /// Returns a snapshot of predictor statistics.
    pub fn stats(&self) -> MdpStats {
        self.stats.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, MemDepPredictor as MdpType, StoreSetConfig};

    fn blind_config() -> Config {
        let mut c = Config::default();
        c.pipeline.mem_dep_predictor = MdpType::Blind;
        c
    }

    fn store_set_config() -> Config {
        let mut c = Config::default();
        c.pipeline.mem_dep_predictor = MdpType::StoreSet;
        c.pipeline.store_set =
            StoreSetConfig { ssit_size: 64, lfst_size: 16, ssit_clear_interval: 0 };
        c
    }

    #[test]
    fn test_blind_dispatch_loads_wait_all() {
        let config = blind_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(mdu.dispatch(0x1000, RobTag(1), true, false), MemDepState::WaitAll);
        assert_eq!(mdu.stats().predictions_wait_all, 1);
    }

    #[test]
    fn test_blind_dispatch_stores_none() {
        let config = blind_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(mdu.dispatch(0x2000, RobTag(2), false, true), MemDepState::None);
    }

    #[test]
    fn test_blind_dispatch_non_memory_none() {
        let config = blind_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(mdu.dispatch(0x3000, RobTag(3), false, false), MemDepState::None);
    }

    #[test]
    fn test_store_set_unknown_pc_bypass() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(mdu.dispatch(0x1000, RobTag(1), true, false), MemDepState::Bypass);
        assert_eq!(mdu.stats().predictions_bypass, 1);
    }

    #[test]
    fn test_store_set_trained_dep() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        // Train violation.
        mdu.violation(load_pc, store_pc);

        // Dispatch store — registers in LFST.
        let s1 = RobTag(5);
        assert_eq!(mdu.dispatch(store_pc, s1, false, true), MemDepState::None);

        // Dispatch load — depends on store.
        let l1 = RobTag(10);
        assert_eq!(mdu.dispatch(load_pc, l1, true, false), MemDepState::WaitFor(s1));
        assert_eq!(mdu.stats().predictions_wait_for, 1);
    }

    #[test]
    fn test_store_resolved_wakeup() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        mdu.violation(load_pc, store_pc);

        let s1 = RobTag(5);
        let _ = mdu.dispatch(store_pc, s1, false, true);

        let l1 = RobTag(10);
        let _ = mdu.dispatch(load_pc, l1, true, false);

        // Resolve store — should wake the load.
        let woken = mdu.store_resolved(s1);
        assert_eq!(woken, Some(s1));
    }

    #[test]
    fn test_issued_cleans_up() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        mdu.violation(load_pc, store_pc);
        let s1 = RobTag(5);
        let _ = mdu.dispatch(store_pc, s1, false, true);
        let l1 = RobTag(10);
        let _ = mdu.dispatch(load_pc, l1, true, false);

        // Issue the load — dep record removed.
        mdu.issued(l1);
        assert!(mdu.deps.is_empty());
    }

    #[test]
    fn test_flush_clears_deps() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        mdu.violation(load_pc, store_pc);
        let s1 = RobTag(5);
        let _ = mdu.dispatch(store_pc, s1, false, true);
        let l1 = RobTag(10);
        let _ = mdu.dispatch(load_pc, l1, true, false);

        mdu.flush();
        assert!(mdu.deps.is_empty());
    }

    #[test]
    fn test_younger_barrier_ignored() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        mdu.violation(load_pc, store_pc);

        // Dispatch load FIRST (older), then store (younger).
        let l1 = RobTag(1);
        assert_eq!(mdu.dispatch(load_pc, l1, true, false), MemDepState::Bypass);

        let s1 = RobTag(5);
        let _ = mdu.dispatch(store_pc, s1, false, true);

        // Dispatch another load — LFST points to s1 which is younger, so NoDep for
        // loads older than s1... but l2 is newer than s1, so DepOn(s1).
        let l2 = RobTag(10);
        assert_eq!(mdu.dispatch(load_pc, l2, true, false), MemDepState::WaitFor(s1));
    }
}

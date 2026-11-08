//! Memory Dependence Unit — gem5-style dependency tracker with wakeup.
//!
//! Owns a predictor (`Blind` or `StoreSet`) and maintains a per-instruction
//! dependency map. The pipeline queries this unit at dispatch to get a
//! cached [`MemDepState`] for each instruction, and notifies it when
//! stores resolve so that waiting instructions can be woken.

use std::collections::HashMap;

use crate::common::VirtAddr;
use crate::config::{Config, MemDepPredictorKind};
use crate::exec::signals::ControlSignals;
use crate::uarch::pipeline::rob::{Rob, RobTag};
use crate::uarch::pipeline::squash::SquashScope;

use super::predictor::{MdpStats, MemPrediction};
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

/// The memory access an instruction makes, as the MDP orders it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemDepAccess {
    /// None the MDP orders.
    None,
    /// A load.
    Load,
    /// A store or a cache-block operation: one the store buffer holds.
    Store,
    /// An LR, SC or AMO. It waits for every older store and neither trains
    /// nor chains through the store sets: an atomic must observe all prior
    /// memory in program order, and chaining atomics through SSIT/LFST
    /// can deadlock on an older one in the same set that has not drained.
    Atomic,
}

impl MemDepAccess {
    /// The access `ctrl` makes.
    pub const fn of(ctrl: &ControlSignals) -> Self {
        if ctrl.atomic_op.is_some() {
            Self::Atomic
        } else if ctrl.mem_read {
            Self::Load
        } else if ctrl.uses_store_buffer() {
            Self::Store
        } else {
            Self::None
        }
    }
}

/// Memory Dependence Unit — owns a predictor and dependency tracking.
#[derive(Debug)]
pub struct MemDepUnit {
    predictor: PredictorKind,
    /// The instructions still waiting on a store, each with the store it waits on.
    deps: HashMap<RobTag, RobTag>,
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

impl MemDepUnit {
    /// Creates a new `MemDepUnit` from configuration.
    pub fn new(config: &Config) -> Self {
        let predictor = match config.pipeline.mem_dep_predictor {
            MemDepPredictorKind::Blind => PredictorKind::Blind,
            MemDepPredictorKind::StoreSet => PredictorKind::StoreSet(Box::new(
                StoreSetPredictor::new(&config.pipeline.store_set),
            )),
        };
        Self { predictor, deps: HashMap::new(), stats: MdpStats::default() }
    }

    /// Called at dispatch for every instruction: the [`MemDepState`] to
    /// cache in its IQ entry. A store is also registered in the LFST, after
    /// it is looked up for its own chain predecessor.
    pub fn dispatch(&mut self, pc: VirtAddr, rob_tag: RobTag, access: MemDepAccess) -> MemDepState {
        match (&mut self.predictor, access) {
            (_, MemDepAccess::None) | (PredictorKind::Blind, MemDepAccess::Store) => {
                MemDepState::None
            }
            (PredictorKind::Blind, MemDepAccess::Load | MemDepAccess::Atomic) => {
                self.stats.predictions_wait_all += 1;
                MemDepState::WaitAll
            }
            (PredictorKind::StoreSet(predictor), MemDepAccess::Atomic) => {
                predictor.note_mem_op();
                self.stats.predictions_wait_all += 1;
                MemDepState::WaitAll
            }
            (PredictorKind::StoreSet(predictor), MemDepAccess::Load | MemDepAccess::Store) => {
                predictor.note_mem_op();
                let prediction = predictor.predict(pc);
                if access == MemDepAccess::Store {
                    predictor.register_store(pc, rob_tag);
                }
                match prediction {
                    MemPrediction::DepOn(barrier) if barrier.is_older_than(rob_tag) => {
                        let _ = self.deps.insert(rob_tag, barrier);
                        self.stats.predictions_wait_for += 1;
                        MemDepState::WaitFor(barrier)
                    }
                    // No dependence, or a younger barrier: an LFST alias.
                    _ if access == MemDepAccess::Load => {
                        self.stats.predictions_bypass += 1;
                        MemDepState::Bypass
                    }
                    _ => MemDepState::None,
                }
            }
        }
    }

    /// Called when a store resolves its address (in memory2).
    ///
    /// Drops the dependences on this store. Returns the store's `RobTag`
    /// when something waited on it, so the IQ can wake entries whose
    /// `WaitFor(barrier)` matches it.
    pub fn store_resolved(&mut self, store_rob_tag: RobTag) -> Option<RobTag> {
        let waiting = self.deps.len();
        self.deps.retain(|_, barrier| *barrier != store_rob_tag);
        (self.deps.len() < waiting).then_some(store_rob_tag)
    }

    /// Called when an instruction is issued from the IQ.
    ///
    /// Removes its dependency record (cleanup).
    pub fn issued(&mut self, rob_tag: RobTag) {
        let _ = self.deps.remove(&rob_tag);
    }

    /// Train on violation detection.
    pub fn violation(&mut self, load_pc: VirtAddr, store_pc: VirtAddr) {
        self.stats.violations += 1;
        if let PredictorKind::StoreSet(predictor) = &mut self.predictor {
            predictor.train(load_pc, store_pc);
        }
    }

    /// Drops what `scope` squashes. Must follow the ROB's squash, so the
    /// ROB holds only survivors when the LFST is rebuilt from it.
    pub fn squash(&mut self, scope: SquashScope, rob: &Rob) {
        match scope {
            SquashScope::After(keep) => self.flush_after(keep, rob),
            SquashScope::All => self.flush(),
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
        self.deps.retain(|&tag, _| tag.is_older_or_eq(keep_tag));
        if let PredictorKind::StoreSet(predictor) = &mut self.predictor {
            predictor.flush_after(keep_tag);
            for entry in rob.iter() {
                if entry.ctrl.mem_write {
                    predictor.rebuild_lfst_entry(entry.pc, entry.tag);
                }
            }
        }
    }

    /// Returns a snapshot of predictor statistics.
    #[cfg(test)]
    pub fn stats(&self) -> MdpStats {
        self.stats.clone()
    }

    /// The counts since the last call, which start again from zero.
    pub fn take_stats(&mut self) -> MdpStats {
        std::mem::take(&mut self.stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, MemDepPredictorKind, StoreSetConfig};

    fn blind_config() -> Config {
        let mut c = Config::minimal();
        c.pipeline.mem_dep_predictor = MemDepPredictorKind::Blind;
        c
    }

    fn store_set_config() -> Config {
        let mut c = Config::minimal();
        c.pipeline.mem_dep_predictor = MemDepPredictorKind::StoreSet;
        c.pipeline.store_set = StoreSetConfig { ssit_size: 64, lfst_size: 16, clear_period: 0 };
        c
    }

    #[test]
    fn a_blind_predictor_makes_loads_wait_for_all_older_stores() {
        let config = blind_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(
            mdu.dispatch(VirtAddr::new(0x1000), RobTag::new(1), MemDepAccess::Load),
            MemDepState::WaitAll
        );
        assert_eq!(mdu.stats().predictions_wait_all, 1);
    }

    #[test]
    fn a_blind_predictor_gives_stores_no_dependence() {
        let config = blind_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(
            mdu.dispatch(VirtAddr::new(0x2000), RobTag::new(2), MemDepAccess::Store),
            MemDepState::None
        );
    }

    #[test]
    fn a_blind_predictor_gives_non_memory_instructions_no_dependence() {
        let config = blind_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(
            mdu.dispatch(VirtAddr::new(0x3000), RobTag::new(3), MemDepAccess::None),
            MemDepState::None
        );
    }

    #[test]
    fn an_atomic_waits_for_all_older_stores_under_either_predictor() {
        for config in [blind_config(), store_set_config()] {
            let mut mdu = MemDepUnit::new(&config);

            let state = mdu.dispatch(VirtAddr::new(0x4000), RobTag::new(4), MemDepAccess::Atomic);

            assert_eq!(state, MemDepState::WaitAll);
            assert_eq!(mdu.stats().predictions_wait_all, 1);
        }
    }

    #[test]
    fn a_load_with_an_untrained_pc_bypasses_older_stores() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        assert_eq!(
            mdu.dispatch(VirtAddr::new(0x1000), RobTag::new(1), MemDepAccess::Load),
            MemDepState::Bypass
        );
        assert_eq!(mdu.stats().predictions_bypass, 1);
    }

    #[test]
    fn a_load_trained_against_a_store_waits_for_it() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = VirtAddr::new(0x1000);
        let store_pc = VirtAddr::new(0x2000);

        // Train violation.
        mdu.violation(load_pc, store_pc);

        // Dispatch store — registers in LFST.
        let s1 = RobTag::new(5);
        assert_eq!(mdu.dispatch(store_pc, s1, MemDepAccess::Store), MemDepState::None);

        // Dispatch load — depends on store.
        let l1 = RobTag::new(10);
        assert_eq!(mdu.dispatch(load_pc, l1, MemDepAccess::Load), MemDepState::WaitFor(s1));
        assert_eq!(mdu.stats().predictions_wait_for, 1);
    }

    #[test]
    fn a_store_wakes_its_waiters_only_the_first_time_it_resolves() {
        let mut mdu = MemDepUnit::new(&store_set_config());
        let (store_pc, load_pc) = (VirtAddr::new(0x2000), VirtAddr::new(0x1000));
        mdu.violation(load_pc, store_pc);
        let store = RobTag::new(5);
        let _ = mdu.dispatch(store_pc, store, MemDepAccess::Store);
        let _ = mdu.dispatch(load_pc, RobTag::new(10), MemDepAccess::Load);

        let first = mdu.store_resolved(store);
        let second = mdu.store_resolved(store);

        assert_eq!((first, second), (Some(store), None));
    }

    #[test]
    fn issuing_a_load_drops_its_dependence() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = VirtAddr::new(0x1000);
        let store_pc = VirtAddr::new(0x2000);

        mdu.violation(load_pc, store_pc);
        let s1 = RobTag::new(5);
        let _ = mdu.dispatch(store_pc, s1, MemDepAccess::Store);
        let l1 = RobTag::new(10);
        let _ = mdu.dispatch(load_pc, l1, MemDepAccess::Load);

        // Issue the load — dep record removed.
        mdu.issued(l1);
        assert!(mdu.deps.is_empty());
    }

    #[test]
    fn a_flush_drops_every_dependence() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = VirtAddr::new(0x1000);
        let store_pc = VirtAddr::new(0x2000);

        mdu.violation(load_pc, store_pc);
        let s1 = RobTag::new(5);
        let _ = mdu.dispatch(store_pc, s1, MemDepAccess::Store);
        let l1 = RobTag::new(10);
        let _ = mdu.dispatch(load_pc, l1, MemDepAccess::Load);

        mdu.flush();
        assert!(mdu.deps.is_empty());
    }

    #[test]
    fn a_load_waits_only_for_a_predicted_store_older_than_it() {
        let config = store_set_config();
        let mut mdu = MemDepUnit::new(&config);
        let load_pc = VirtAddr::new(0x1000);
        let store_pc = VirtAddr::new(0x2000);
        mdu.violation(load_pc, store_pc);
        let store = RobTag::new(5);
        let _ = mdu.dispatch(store_pc, store, MemDepAccess::Store);

        let older_load = mdu.dispatch(load_pc, RobTag::new(3), MemDepAccess::Load);
        let younger_load = mdu.dispatch(load_pc, RobTag::new(10), MemDepAccess::Load);

        assert_eq!((older_load, younger_load), (MemDepState::Bypass, MemDepState::WaitFor(store)));
    }
}

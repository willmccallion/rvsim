//! Store-set memory dependence predictor (Chrysos & Emer, ISCA 1998).
//!
//! Learns load-store dependencies from ordering violations. Loads predicted
//! independent bypass unresolved stores; loads in a known store set wait only
//! for the specific linked store. Store-store chains are supported: a new store
//! in a set waits for the previous store in the same set.
//!
//! # Structures
//!
//! - **SSIT** (Store Set ID Table): maps PC → store set ID. Indexed by
//!   `(pc >> 2) % ssit_size`. Persistent across flushes; both tables are
//!   wiped every `clear_period` memory instructions, as gem5's
//!   `StoreSet::checkClear` does.
//! - **LFST** (Last Fetched Store Table): maps store set ID → most recent
//!   dispatched store's [`RobTag`]. Transient — cleared on full flush.
//!   LFST entries persist until overwritten by a newer store dispatch
//!   (the [`MemDepUnit`](super::mem_dep_unit::MemDepUnit) handles wakeup,
//!   not LFST clearing).

use crate::config::StoreSetConfig;
use crate::core::pipeline::rob::RobTag;

use super::mem_dep_predictor::{MemDepPredictor, MemPrediction};
use super::types::{SsitIndex, StoreSetId};

/// Store-set memory dependence predictor.
#[derive(Debug)]
pub struct StoreSetPredictor {
    /// Maps PC → store set ID. `None` means no known dependency.
    ssit: Vec<Option<StoreSetId>>,
    /// Maps store set ID → most recent dispatched store tag.
    lfst: Vec<Option<RobTag>>,
    /// Memory instructions seen since the tables were last wiped.
    mem_ops: u64,
    /// Memory instructions between wipes (0 = never).
    clear_period: u64,
}

impl StoreSetPredictor {
    /// Creates a new store-set predictor from configuration.
    pub fn new(config: &StoreSetConfig) -> Self {
        Self {
            ssit: vec![None; config.ssit_size],
            lfst: vec![None; config.lfst_size],
            mem_ops: 0,
            clear_period: config.clear_period,
        }
    }

    /// The set a new load/store pair starts, derived from the load's PC as
    /// gem5's `calcSSID` does.
    const fn new_set_id(&self, load_pc: u64) -> StoreSetId {
        StoreSetId(((load_pc ^ (load_pc >> 10)) % self.lfst.len() as u64) as u16)
    }

    /// Hash a PC into an SSIT index.
    #[inline]
    const fn ssit_index(&self, pc: u64) -> SsitIndex {
        SsitIndex::from_pc(pc, self.ssit.len())
    }
}

impl MemDepPredictor for StoreSetPredictor {
    fn predict(&mut self, pc: u64, _rob_tag: RobTag, _is_store: bool) -> MemPrediction {
        let idx = self.ssit_index(pc);
        let Some(set_id) = self.ssit[idx.0 as usize] else {
            return MemPrediction::NoDep;
        };
        self.lfst[set_id.0 as usize].map_or(MemPrediction::NoDep, MemPrediction::DepOn)
    }

    fn register_store(&mut self, store_pc: u64, rob_tag: RobTag) {
        let idx = self.ssit_index(store_pc);
        if let Some(set_id) = self.ssit[idx.0 as usize] {
            self.lfst[set_id.0 as usize] = Some(rob_tag);
        }
    }

    fn train(&mut self, load_pc: u64, store_pc: u64) {
        let load_idx = self.ssit_index(load_pc);
        let store_idx = self.ssit_index(store_pc);

        let load_set = self.ssit[load_idx.0 as usize];
        let store_set = self.ssit[store_idx.0 as usize];

        match (load_set, store_set) {
            (None, None) => {
                let id = self.new_set_id(load_pc);
                self.ssit[load_idx.0 as usize] = Some(id);
                self.ssit[store_idx.0 as usize] = Some(id);
            }
            (None, Some(sid)) => {
                self.ssit[load_idx.0 as usize] = Some(sid);
            }
            (Some(lid), None) => {
                self.ssit[store_idx.0 as usize] = Some(lid);
            }
            (Some(lid), Some(sid)) => {
                // The lower-numbered set wins, as in gem5.
                if sid > lid {
                    self.ssit[store_idx.0 as usize] = Some(lid);
                } else {
                    self.ssit[load_idx.0 as usize] = Some(sid);
                }
            }
        }
    }

    fn rebuild_lfst_entry(&mut self, store_pc: u64, rob_tag: RobTag) {
        let idx = self.ssit_index(store_pc);
        if let Some(set_id) = self.ssit[idx.0 as usize] {
            let slot = &mut self.lfst[set_id.0 as usize];
            match *slot {
                None => *slot = Some(rob_tag),
                Some(current) => {
                    if rob_tag.is_newer_than(current) {
                        *slot = Some(rob_tag);
                    }
                }
            }
        }
    }

    fn flush(&mut self) {
        self.lfst.fill(None);
    }

    fn flush_after(&mut self, keep_tag: RobTag) {
        for entry in &mut self.lfst {
            if let Some(tag) = *entry
                && tag.is_newer_than(keep_tag)
            {
                *entry = None;
            }
        }
    }

    fn note_mem_op(&mut self) {
        self.mem_ops += 1;
        if self.clear_period > 0 && self.mem_ops > self.clear_period {
            self.mem_ops = 0;
            self.ssit.fill(None);
            self.lfst.fill(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreSetConfig;

    fn test_predictor() -> StoreSetPredictor {
        StoreSetPredictor::new(&StoreSetConfig { ssit_size: 64, lfst_size: 16, clear_period: 0 })
    }

    #[test]
    fn test_unknown_pc_returns_no_dep() {
        let mut p = test_predictor();
        assert_eq!(p.predict(0x1000, RobTag(1), false), MemPrediction::NoDep);
        assert_eq!(p.predict(0x2000, RobTag(2), false), MemPrediction::NoDep);
    }

    #[test]
    fn test_train_creates_set_then_dep_on() {
        let mut p = test_predictor();
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        // Train a violation — creates SSIT entries for both.
        p.train(load_pc, store_pc);

        // SSIT has entries but LFST empty → NoDep.
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::NoDep);

        // Dispatch a store — LFST populated.
        let store_tag = RobTag(5);
        p.register_store(store_pc, store_tag);
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::DepOn(store_tag));
    }

    #[test]
    fn test_store_store_chain() {
        let mut p = test_predictor();
        let store_pc = 0x2000;
        let load_pc = 0x1000;

        // Train: load violated against store.
        p.train(load_pc, store_pc);

        // Dispatch store S1 — predict() returns NoDep (no prior store in set),
        // then register_store populates LFST.
        let s1 = RobTag(1);
        assert_eq!(p.predict(store_pc, s1, true), MemPrediction::NoDep);
        p.register_store(store_pc, s1);

        // Dispatch store S2 — predict() returns DepOn(S1) (chain predecessor),
        // then register_store overwrites LFST with S2.
        let s2 = RobTag(2);
        assert_eq!(p.predict(store_pc, s2, true), MemPrediction::DepOn(s1));
        p.register_store(store_pc, s2);

        // Dispatch load — returns DepOn(S2) (most recent store in set).
        let l1 = RobTag(3);
        assert_eq!(p.predict(load_pc, l1, false), MemPrediction::DepOn(s2));
    }

    #[test]
    fn test_flush_clears_lfst_keeps_ssit() {
        let mut p = test_predictor();
        let load_pc = 0x1000;
        let store_pc = 0x2000;
        let store_tag = RobTag(5);

        p.train(load_pc, store_pc);
        p.register_store(store_pc, store_tag);
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::DepOn(store_tag));

        // Full flush — LFST cleared, SSIT persists.
        p.flush();
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::NoDep);

        // Dispatch a new store — SSIT still maps, so LFST gets repopulated.
        let new_tag = RobTag(10);
        p.register_store(store_pc, new_tag);
        assert_eq!(p.predict(load_pc, RobTag(15), false), MemPrediction::DepOn(new_tag));
    }

    #[test]
    fn test_flush_after_partial() {
        let mut p = test_predictor();
        // PCs chosen to produce distinct SSIT indices: (pc >> 2) % 64.
        let store_pc_a = 0x1004; // index 1
        let store_pc_b = 0x1008; // index 2
        let load_pc_a = 0x100C; // index 3
        let load_pc_b = 0x1010; // index 4

        p.train(load_pc_a, store_pc_a);
        p.train(load_pc_b, store_pc_b);

        let old_tag = RobTag(2);
        let new_tag = RobTag(8);
        p.register_store(store_pc_a, old_tag);
        p.register_store(store_pc_b, new_tag);

        // Flush after tag 5 — old_tag(2) survives, new_tag(8) is cleared.
        p.flush_after(RobTag(5));

        assert_eq!(p.predict(load_pc_a, RobTag(10), false), MemPrediction::DepOn(old_tag));
        assert_eq!(p.predict(load_pc_b, RobTag(10), false), MemPrediction::NoDep);
    }

    #[test]
    fn test_merge_sets() {
        let mut p = test_predictor();
        let load_pc = 0x1000;
        let store_a = 0x2000;
        let store_b = 0x3000;

        // Create two separate store sets.
        p.train(load_pc, store_a);
        p.train(0x4000, store_b);

        // Now violate: load_pc conflicts with store_b too.
        // This should merge load_pc into store_b's set.
        p.train(load_pc, store_b);

        let tag_b = RobTag(10);
        p.register_store(store_b, tag_b);
        assert_eq!(p.predict(load_pc, RobTag(15), false), MemPrediction::DepOn(tag_b));
    }

    #[test]
    fn test_rebuild_lfst_entry() {
        let mut p = test_predictor();
        let store_pc = 0x2000;
        let load_pc = 0x1000;

        p.train(load_pc, store_pc);

        // Rebuild with tag 5.
        p.rebuild_lfst_entry(store_pc, RobTag(5));
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::DepOn(RobTag(5)));

        // Rebuild with older tag 3 — should NOT overwrite (tag 5 is newer).
        p.rebuild_lfst_entry(store_pc, RobTag(3));
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::DepOn(RobTag(5)));

        // Rebuild with newer tag 8 — should overwrite.
        p.rebuild_lfst_entry(store_pc, RobTag(8));
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::DepOn(RobTag(8)));
    }

    #[test]
    fn tables_are_wiped_after_the_clear_period_of_memory_ops() {
        let mut p = StoreSetPredictor::new(&StoreSetConfig {
            ssit_size: 64,
            lfst_size: 16,
            clear_period: 100,
        });
        let load_pc = 0x1000;
        let store_pc = 0x2000;
        p.train(load_pc, store_pc);
        p.register_store(store_pc, RobTag(5));

        for _ in 0..100 {
            p.note_mem_op();
        }
        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::DepOn(RobTag(5)));
        p.note_mem_op();

        assert_eq!(p.predict(load_pc, RobTag(10), false), MemPrediction::NoDep);
        p.register_store(store_pc, RobTag(11));
        assert_eq!(p.predict(load_pc, RobTag(12), false), MemPrediction::NoDep);
    }

    #[test]
    fn a_new_set_is_numbered_from_the_load_pc() {
        let mut p = test_predictor();
        let load_pc: u64 = 0x1234_5678;

        p.train(load_pc, 0x2000);

        let expected = StoreSetId(((load_pc ^ (load_pc >> 10)) % 16) as u16);
        assert_eq!(p.ssit[p.ssit_index(load_pc).0 as usize], Some(expected));
        assert_eq!(p.ssit[p.ssit_index(0x2000).0 as usize], Some(expected));
    }

    #[test]
    fn merging_two_sets_keeps_the_lower_id() {
        let mut p = test_predictor();
        let (load_a, store_a) = (0x1004, 0x2004);
        let (load_b, store_b) = (0x1010, 0x2010);
        p.train(load_a, store_a);
        p.train(load_b, store_b);
        let set_a = p.ssit[p.ssit_index(load_a).0 as usize].unwrap();
        let set_b = p.ssit[p.ssit_index(load_b).0 as usize].unwrap();
        assert_ne!(set_a, set_b);

        p.train(load_a, store_b);

        let lower = set_a.min(set_b);
        assert_eq!(p.ssit[p.ssit_index(load_a).0 as usize], Some(lower));
        assert_eq!(p.ssit[p.ssit_index(store_b).0 as usize], Some(lower));
    }
}

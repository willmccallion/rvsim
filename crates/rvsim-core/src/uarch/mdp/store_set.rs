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
//!   (the [`MemDepUnit`](super::unit::MemDepUnit) handles wakeup,
//!   not LFST clearing).

use crate::common::ids::impl_id_from_index;
use crate::common::usize_of;
use crate::config::StoreSetConfig;
use crate::uarch::pipeline::rob::RobTag;

use super::predictor::MemPrediction;

/// Store Set ID — indexes into the LFST.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreSetId(pub u16);

impl_id_from_index!(StoreSetId, u16);

/// Index into the SSIT table. Derived from PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SsitIndex(pub usize);

impl SsitIndex {
    /// Compute the SSIT index for a given PC and table size.
    #[inline]
    pub const fn from_pc(pc: u64, table_size: usize) -> Self {
        Self(usize_of(pc >> 2) % table_size)
    }
}

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
    fn new_set_id(&self, load_pc: u64) -> StoreSetId {
        StoreSetId::from_index(usize_of(load_pc ^ (load_pc >> 10)) % self.lfst.len())
    }

    /// Hash a PC into an SSIT index.
    #[inline]
    const fn ssit_index(&self, pc: u64) -> SsitIndex {
        SsitIndex::from_pc(pc, self.ssit.len())
    }
}

impl StoreSetPredictor {
    /// Query SSIT+LFST for a dependency. Called once at dispatch.
    ///
    /// For stores, returns the previous store in the same set (chain predecessor)
    /// before [`register_store`](Self::register_store) overwrites the LFST entry.
    pub fn predict(&self, pc: u64) -> MemPrediction {
        let idx = self.ssit_index(pc);
        let Some(set_id) = self.ssit[idx.0] else {
            return MemPrediction::NoDep;
        };
        self.lfst[set_id.0 as usize].map_or(MemPrediction::NoDep, MemPrediction::DepOn)
    }

    /// Register a store in the LFST. Called AFTER `predict()` for the same store,
    /// so `predict()` sees the previous store, not itself.
    pub fn register_store(&mut self, store_pc: u64, rob_tag: RobTag) {
        let idx = self.ssit_index(store_pc);
        if let Some(set_id) = self.ssit[idx.0] {
            self.lfst[set_id.0 as usize] = Some(rob_tag);
        }
    }

    /// Train: a load at `load_pc` violated against store at `store_pc`.
    pub fn train(&mut self, load_pc: u64, store_pc: u64) {
        let load_idx = self.ssit_index(load_pc);
        let store_idx = self.ssit_index(store_pc);

        let load_set = self.ssit[load_idx.0];
        let store_set = self.ssit[store_idx.0];

        match (load_set, store_set) {
            (None, None) => {
                let id = self.new_set_id(load_pc);
                self.ssit[load_idx.0] = Some(id);
                self.ssit[store_idx.0] = Some(id);
            }
            (None, Some(sid)) => {
                self.ssit[load_idx.0] = Some(sid);
            }
            (Some(lid), None) => {
                self.ssit[store_idx.0] = Some(lid);
            }
            (Some(lid), Some(sid)) => {
                // The lower-numbered set wins, as in gem5.
                if sid > lid {
                    self.ssit[store_idx.0] = Some(lid);
                } else {
                    self.ssit[load_idx.0] = Some(sid);
                }
            }
        }
    }

    /// Rebuild a single LFST entry from a surviving store after partial flush.
    /// Only overwrites if `rob_tag` is newer than the current entry.
    pub fn rebuild_lfst_entry(&mut self, store_pc: u64, rob_tag: RobTag) {
        let idx = self.ssit_index(store_pc);
        if let Some(set_id) = self.ssit[idx.0] {
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

    /// Full flush — clear LFST (SSIT persists).
    pub fn flush(&mut self) {
        self.lfst.fill(None);
    }

    /// Partial flush — clear LFST entries newer than `keep_tag`.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for entry in &mut self.lfst {
            if let Some(tag) = *entry
                && tag.is_newer_than(keep_tag)
            {
                *entry = None;
            }
        }
    }

    /// Counts one dispatched load or store toward the periodic table wipe.
    pub fn note_mem_op(&mut self) {
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
    fn an_untrained_pc_predicts_no_dependence() {
        let p = test_predictor();
        assert_eq!(p.predict(0x1000), MemPrediction::NoDep);
        assert_eq!(p.predict(0x2000), MemPrediction::NoDep);
    }

    #[test]
    fn a_trained_load_depends_on_its_sets_store_once_registered() {
        let mut p = test_predictor();
        let load_pc = 0x1000;
        let store_pc = 0x2000;

        // Train a violation — creates SSIT entries for both.
        p.train(load_pc, store_pc);

        // SSIT has entries but LFST empty → NoDep.
        assert_eq!(p.predict(load_pc), MemPrediction::NoDep);

        // Dispatch a store — LFST populated.
        let store_tag = RobTag::new(5);
        p.register_store(store_pc, store_tag);
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(store_tag));
    }

    #[test]
    fn a_store_depends_on_the_previous_store_in_its_set() {
        let mut p = test_predictor();
        let store_pc = 0x2000;
        let load_pc = 0x1000;

        // Train: load violated against store.
        p.train(load_pc, store_pc);

        // Dispatch store S1 — predict() returns NoDep (no prior store in set),
        // then register_store populates LFST.
        let s1 = RobTag::new(1);
        assert_eq!(p.predict(store_pc), MemPrediction::NoDep);
        p.register_store(store_pc, s1);

        // Dispatch store S2 — predict() returns DepOn(S1) (chain predecessor),
        // then register_store overwrites LFST with S2.
        let s2 = RobTag::new(2);
        assert_eq!(p.predict(store_pc), MemPrediction::DepOn(s1));
        p.register_store(store_pc, s2);

        // Dispatch load — returns DepOn(S2) (most recent store in set).
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(s2));
    }

    #[test]
    fn a_full_flush_clears_the_lfst_but_keeps_the_ssit() {
        let mut p = test_predictor();
        let load_pc = 0x1000;
        let store_pc = 0x2000;
        let store_tag = RobTag::new(5);

        p.train(load_pc, store_pc);
        p.register_store(store_pc, store_tag);
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(store_tag));

        // Full flush — LFST cleared, SSIT persists.
        p.flush();
        assert_eq!(p.predict(load_pc), MemPrediction::NoDep);

        // Dispatch a new store — SSIT still maps, so LFST gets repopulated.
        let new_tag = RobTag::new(10);
        p.register_store(store_pc, new_tag);
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(new_tag));
    }

    #[test]
    fn flush_after_clears_only_lfst_entries_newer_than_the_tag() {
        let mut p = test_predictor();
        // PCs chosen to produce distinct SSIT indices: (pc >> 2) % 64.
        let store_pc_a = 0x1004; // index 1
        let store_pc_b = 0x1008; // index 2
        let load_pc_a = 0x100C; // index 3
        let load_pc_b = 0x1010; // index 4

        p.train(load_pc_a, store_pc_a);
        p.train(load_pc_b, store_pc_b);

        let old_tag = RobTag::new(2);
        let new_tag = RobTag::new(8);
        p.register_store(store_pc_a, old_tag);
        p.register_store(store_pc_b, new_tag);

        // Flush after tag 5 — old_tag(2) survives, new_tag(8) is cleared.
        p.flush_after(RobTag::new(5));

        assert_eq!(p.predict(load_pc_a), MemPrediction::DepOn(old_tag));
        assert_eq!(p.predict(load_pc_b), MemPrediction::NoDep);
    }

    #[test]
    fn a_load_trained_against_a_second_store_depends_on_it() {
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

        let tag_b = RobTag::new(10);
        p.register_store(store_b, tag_b);
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(tag_b));
    }

    #[test]
    fn rebuilding_an_lfst_entry_keeps_the_newest_store() {
        let mut p = test_predictor();
        let store_pc = 0x2000;
        let load_pc = 0x1000;

        p.train(load_pc, store_pc);

        // Rebuild with tag 5.
        p.rebuild_lfst_entry(store_pc, RobTag::new(5));
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(RobTag::new(5)));

        // Rebuild with older tag 3 — should NOT overwrite (tag 5 is newer).
        p.rebuild_lfst_entry(store_pc, RobTag::new(3));
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(RobTag::new(5)));

        // Rebuild with newer tag 8 — should overwrite.
        p.rebuild_lfst_entry(store_pc, RobTag::new(8));
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(RobTag::new(8)));
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
        p.register_store(store_pc, RobTag::new(5));

        for _ in 0..100 {
            p.note_mem_op();
        }
        assert_eq!(p.predict(load_pc), MemPrediction::DepOn(RobTag::new(5)));
        p.note_mem_op();

        assert_eq!(p.predict(load_pc), MemPrediction::NoDep);
        p.register_store(store_pc, RobTag::new(11));
        assert_eq!(p.predict(load_pc), MemPrediction::NoDep);
    }

    #[test]
    fn a_high_pc_indexes_the_ssit_by_its_whole_word_index() {
        let pc = 0xffff_ffff_8000_0000_u64;

        let index = SsitIndex::from_pc(pc, 1000);

        assert_eq!(index, SsitIndex(usize_of(pc >> 2) % 1000));
        assert_eq!(index, SsitIndex(992));
    }

    #[test]
    fn a_new_set_is_numbered_from_the_load_pc() {
        let mut p = test_predictor();
        let load_pc: u64 = 0x1234_5678;

        p.train(load_pc, 0x2000);

        let expected = StoreSetId(((load_pc ^ (load_pc >> 10)) % 16) as u16);
        assert_eq!(p.ssit[p.ssit_index(load_pc).0], Some(expected));
        assert_eq!(p.ssit[p.ssit_index(0x2000).0], Some(expected));
    }

    #[test]
    fn merging_two_sets_keeps_the_lower_id() {
        let mut p = test_predictor();
        let (load_a, store_a) = (0x1004, 0x2004);
        let (load_b, store_b) = (0x1010, 0x2010);
        p.train(load_a, store_a);
        p.train(load_b, store_b);
        let set_a = p.ssit[p.ssit_index(load_a).0].unwrap();
        let set_b = p.ssit[p.ssit_index(load_b).0].unwrap();
        assert_ne!(set_a, set_b);

        p.train(load_a, store_b);

        let lower = set_a.min(set_b);
        assert_eq!(p.ssit[p.ssit_index(load_a).0], Some(lower));
        assert_eq!(p.ssit[p.ssit_index(store_b).0], Some(lower));
    }
}

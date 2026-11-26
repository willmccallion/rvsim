//! Bit pseudo-LRU (bit-PLRU, or MRU-bit) replacement: one bit per way.
//!
//! An access sets its way's bit; once every bit of a set is set, all but the
//! one just set are cleared. The victim is the lowest way whose bit is clear,
//! a way not touched since the bits last cleared. It approximates LRU with N
//! bits per N-way set, against a tree PLRU's N-1.

use super::{ReplacementPolicy, SetIdx, WayIdx};

/// PLRU Policy state.
#[derive(Debug)]
pub struct PlruPolicy {
    /// Each set's bits: one per way, set by an access to it.
    usage: Vec<u64>,
    /// Number of ways in the cache.
    ways: usize,
}

impl PlruPolicy {
    /// A PLRU policy for a cache of `sets` sets of `ways` ways.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self { usage: vec![0; sets], ways }
    }
}

impl ReplacementPolicy for PlruPolicy {
    /// Sets `way`'s bit, clearing every other once all are set.
    fn update(&mut self, set: SetIdx, way: WayIdx) {
        let (set, way) = (set.index(), way.index());
        let mask = 1 << way;
        self.usage[set] |= mask;

        let all_ones = (1 << self.ways) - 1;
        if (self.usage[set] & all_ones) == all_ones {
            self.usage[set] = mask;
        }
    }

    /// The lowest way whose bit is clear.
    fn get_victim(&mut self, set: SetIdx) -> WayIdx {
        let set = set.index();
        let unused = (0..self.ways).find(|&way| (self.usage[set] >> way) & 1 == 0);
        WayIdx::new(unused.unwrap_or(0))
    }
}

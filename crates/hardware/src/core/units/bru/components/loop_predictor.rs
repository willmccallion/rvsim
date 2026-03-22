//! Standalone Loop Predictor.
//!
//! Detects counted loops and predicts their iteration behavior. When confident
//! in a loop's trip count, it can override any base predictor's decision.

/// An entry in the loop predictor table.
#[derive(Clone, Copy, Debug, Default)]
struct LoopEntry {
    /// PC-derived tag for matching.
    tag: u16,
    /// Current iteration count within the loop.
    current_iter: u16,
    /// Learned total trip count (iterations before exit).
    trip_count: u16,
    /// Confidence: number of confirmed full-trip matches (0-3).
    confidence: u8,
    /// Age counter for replacement (higher = more recently used).
    age: u8,
}

/// Standalone loop predictor that can be composed with any base predictor.
#[derive(Debug)]
pub struct LoopPredictor {
    /// Loop predictor table.
    table: Vec<LoopEntry>,
    /// Size of the loop table (number of entries).
    table_size: usize,
}

impl LoopPredictor {
    /// Creates a new loop predictor with the given table size.
    pub fn new(size: usize) -> Self {
        let size = size.max(1);
        Self { table: vec![LoopEntry::default(); size], table_size: size }
    }

    /// Loop predictor index from PC.
    const fn index(&self, pc: u64) -> usize {
        ((pc >> 2) as usize) % self.table_size
    }

    /// Loop predictor tag from PC (10-bit).
    const fn tag(pc: u64) -> u16 {
        ((pc >> 2) ^ (pc >> 12)) as u16 & 0x3FF
    }

    /// Queries the loop predictor. Returns `Some(taken)` if the loop predictor
    /// has high confidence, otherwise `None`.
    pub fn predict(&self, pc: u64) -> Option<bool> {
        let idx = self.index(pc);
        let entry = &self.table[idx];
        let tag = Self::tag(pc);

        if entry.tag != tag || entry.confidence < 2 || entry.trip_count == 0 {
            return None;
        }

        if entry.current_iter + 1 >= entry.trip_count {
            Some(false) // loop exit
        } else {
            Some(true) // loop body
        }
    }

    /// Updates the loop predictor with actual branch outcome.
    pub fn update(&mut self, pc: u64, taken: bool) {
        let idx = self.index(pc);
        let tag = Self::tag(pc);
        let entry = &mut self.table[idx];

        if entry.tag == tag {
            if taken {
                entry.current_iter = entry.current_iter.saturating_add(1);
                entry.age = entry.age.saturating_add(1).min(3);
            } else {
                // Loop exit.
                if entry.trip_count == 0 {
                    entry.trip_count = entry.current_iter;
                    entry.confidence = 1;
                } else if entry.current_iter == entry.trip_count {
                    entry.confidence = entry.confidence.saturating_add(1).min(3);
                } else {
                    entry.trip_count = entry.current_iter;
                    entry.confidence = 0;
                }
                entry.current_iter = 0;
            }
        } else if taken {
            if entry.age == 0 || entry.tag == 0 {
                *entry = LoopEntry { tag, current_iter: 1, trip_count: 0, confidence: 0, age: 1 };
            } else {
                entry.age = entry.age.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_predictor_basic() {
        let mut lp = LoopPredictor::new(64);
        let pc = 0x8000_1000u64;

        // No prediction initially.
        assert_eq!(lp.predict(pc), None);

        // Simulate a loop with trip count 5 (5 taken, then 1 not-taken).
        // First iteration: learn trip count.
        for _ in 0..5 {
            lp.update(pc, true);
        }
        lp.update(pc, false); // exit — sets trip_count=5, confidence=1

        // Second iteration: confirm trip count.
        for _ in 0..5 {
            lp.update(pc, true);
        }
        lp.update(pc, false); // confidence=2

        // Now it should predict.
        // After the loop exit, current_iter is 0 again. Push 3 taken:
        for _ in 0..3 {
            lp.update(pc, true);
        }
        // current_iter=3, trip_count=5, confidence=2 — should predict taken.
        assert_eq!(lp.predict(pc), Some(true));

        // Push to iteration 4 (current_iter=4).
        lp.update(pc, true);
        // current_iter + 1 = 5 >= trip_count=5 → should predict not-taken (exit).
        assert_eq!(lp.predict(pc), Some(false));
    }
}

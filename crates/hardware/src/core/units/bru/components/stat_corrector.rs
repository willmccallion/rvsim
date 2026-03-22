//! Statistical Corrector (SC) predictor.
//!
//! Based on Seznec's SC design (MICRO 2011, CBP-4/5 winners). A correction layer
//! that can flip a base predictor's direction when it has high confidence that the
//! base prediction is wrong. Uses short history lengths to capture local/short
//! correlations that TAGE misses.
//!
//! Hot-path data uses fixed-size arrays and a flattened counter table to avoid
//! heap indirection during predict/update.

use crate::config::ScConfig;
use crate::core::units::bru::Ghr;

/// Maximum number of SC tables. Covers all realistic configs (Seznec uses 4-6).
const MAX_SC_TABLES: usize = 8;

/// Statistical Corrector predictor.
///
/// Counter tables are stored in a single flat `Vec<i8>` indexed as
/// `[table * table_size + entry]` to eliminate double indirection.
#[derive(Debug)]
pub struct StatCorrector {
    /// Flattened counter tables: `num_tables × table_size` entries.
    counters: Vec<i8>,
    /// Number of active tables.
    num_tables: usize,
    /// History lengths for each table (fixed array).
    hist_lengths: [usize; MAX_SC_TABLES],
    /// Entries per table.
    table_size: usize,
    /// Number of bits for table indexing.
    table_bits: usize,
    /// Mask for indexing the tables.
    table_mask: usize,
    /// Counter bit width (for clamping).
    counter_bits: usize,
    /// Dynamic threshold for correction confidence.
    threshold: i32,
    /// Threshold counter for dynamic adjustment.
    tc: i32,
}

/// Clamps a value to fit within a signed counter of the given bit width.
#[inline]
fn clamp_counter(val: i32, bits: usize) -> i8 {
    let max = (1i32 << (bits - 1)) - 1;
    let min = -(1i32 << (bits - 1));
    val.clamp(min, max) as i8
}

impl StatCorrector {
    /// Creates a new Statistical Corrector from config.
    ///
    /// # Panics
    ///
    /// Panics if `num_tables` exceeds `MAX_SC_TABLES`.
    pub fn new(config: &ScConfig) -> Self {
        assert!(
            config.num_tables <= MAX_SC_TABLES,
            "SC: {} tables exceeds MAX_SC_TABLES ({MAX_SC_TABLES})",
            config.num_tables,
        );

        let table_size = config.table_size.next_power_of_two();
        let table_bits = table_size.trailing_zeros() as usize;

        let mut hist_lengths = [0usize; MAX_SC_TABLES];
        for (i, &hl) in config.history_lengths.iter().enumerate().take(config.num_tables) {
            hist_lengths[i] = hl;
        }

        Self {
            counters: vec![0i8; config.num_tables * table_size],
            num_tables: config.num_tables,
            hist_lengths,
            table_size,
            table_bits,
            table_mask: table_size - 1,
            counter_bits: config.counter_bits,
            threshold: 6,
            tc: 0,
        }
    }

    /// Returns a reference to the counter at `(table, idx)`.
    #[inline]
    fn counter(&self, table: usize, idx: usize) -> i8 {
        self.counters[table * self.table_size + idx]
    }

    /// Returns a mutable reference to the counter at `(table, idx)`.
    #[inline]
    fn counter_mut(&mut self, table: usize, idx: usize) -> &mut i8 {
        &mut self.counters[table * self.table_size + idx]
    }

    /// Computes the table index using word-level XOR-fold of GHR bits.
    /// SC histories are short (≤16 bits), so no CSR needed — but we fold
    /// at word granularity instead of bit-by-bit for speed.
    #[inline]
    const fn table_index(&self, pc: u64, ghr: &Ghr, table: usize) -> usize {
        let pc_hash = (pc >> 2) as usize;
        let hl = self.hist_lengths[table];
        if hl == 0 {
            return pc_hash & self.table_mask;
        }

        // Extract the low `hl` bits from the GHR as a single u64.
        // SC history lengths are ≤16, so this always fits in one word.
        let mask = if hl >= 64 { u64::MAX } else { (1u64 << hl) - 1 };
        let hist_bits = ghr.val() & mask;

        // XOR-fold the extracted bits down to table_bits width.
        let mut h = hist_bits;
        let mut shift = self.table_bits;
        while shift < hl {
            h ^= hist_bits >> shift;
            shift += self.table_bits;
        }

        (pc_hash ^ h as usize) & self.table_mask
    }

    /// Computes the SC prediction. Returns `(corrected_prediction, sc_sum)`.
    ///
    /// `base_taken` is the base predictor's direction, `base_confidence` is
    /// the provider counter's signed value (positive = taken bias, negative = not-taken).
    ///
    /// The base confidence is centered per Seznec's design: `(2*|ctr|+1)` with
    /// sign matching the prediction direction, so even weak predictions (ctr=0)
    /// contribute a non-zero vote to the sum.
    pub fn predict(
        &self,
        pc: u64,
        ghr: &Ghr,
        base_taken: bool,
        base_confidence: i32,
    ) -> (bool, i32) {
        let centered = (2 * base_confidence.abs() + 1) * if base_taken { 1 } else { -1 };
        let mut sum: i32 = centered;

        for t in 0..self.num_tables {
            let idx = self.table_index(pc, ghr, t);
            sum += self.counter(t, idx) as i32;
        }

        let sc_taken = sum >= 0;
        let corrected = if sum.abs() > self.threshold && sc_taken != base_taken {
            sc_taken
        } else {
            base_taken
        };

        (corrected, sum)
    }

    /// Updates the SC tables at commit time.
    pub fn update(&mut self, pc: u64, ghr: &Ghr, taken: bool, base_taken: bool, sc_sum: i32) {
        let sc_taken = sc_sum >= 0;
        let sc_corrected = sc_taken != base_taken && sc_sum.abs() > self.threshold;
        let base_wrong = base_taken != taken;

        if sc_corrected {
            if sc_taken == taken {
                self.tc = (self.tc - 1).max(-32);
            } else {
                self.tc = (self.tc + 1).min(31);
            }
        }

        if self.tc <= -8 {
            self.threshold = (self.threshold - 1).max(2);
            self.tc = 0;
        } else if self.tc >= 8 {
            self.threshold = (self.threshold + 1).min(20);
            self.tc = 0;
        }

        let should_train = base_wrong || sc_corrected || sc_sum.abs() <= self.threshold + 4;
        if !should_train {
            return;
        }

        let bits = self.counter_bits;
        for t in 0..self.num_tables {
            let idx = self.table_index(pc, ghr, t);
            let ctr = self.counter(t, idx) as i32;
            *self.counter_mut(t, idx) =
                if taken { clamp_counter(ctr + 1, bits) } else { clamp_counter(ctr - 1, bits) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ScConfig {
        ScConfig {
            num_tables: 4,
            table_size: 64,
            history_lengths: vec![0, 2, 4, 8],
            counter_bits: 3,
        }
    }

    #[test]
    fn test_sc_initial_prediction_follows_base() {
        let sc = StatCorrector::new(&test_config());
        let ghr = Ghr::with_len(64);

        let (pred, _sum) = sc.predict(0x1000, &ghr, true, 2);
        assert!(pred, "SC should follow base when counters are zero");
    }

    #[test]
    fn test_sc_can_correct() {
        let config = test_config();
        let mut sc = StatCorrector::new(&config);
        let ghr = Ghr::with_len(64);
        let pc = 0x1000u64;

        for _ in 0..100 {
            sc.update(pc, &ghr, false, true, 1);
        }

        let (pred, _sum) = sc.predict(pc, &ghr, true, 1);
        assert!(!pred, "SC should correct weak base prediction after training");
    }
}

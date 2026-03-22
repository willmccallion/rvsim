//! Statistical Corrector (SC) predictor.
//!
//! Faithful implementation of Seznec's CBP-5 Statistical Corrector design.
//! Features: 3 bias tables (PC-indexed with confidence/direction), GEHL tables,
//! multi-tier override logic with FirstH/SecondH choosers, and two-level
//! (global + per-PC) dynamic threshold adaptation.
//!
//! Hot-path data uses fixed-size arrays and flattened counter tables to avoid
//! heap indirection during predict/update.

use crate::config::ScConfig;
use crate::core::units::bru::Ghr;

use super::sc_types::{ScSum, TageConfLevel, TageScMeta};

/// Maximum number of GEHL tables. Covers all realistic configs (Seznec uses 4-6).
const MAX_SC_TABLES: usize = 8;

/// Clamps a value to fit within a signed counter of the given bit width.
#[inline]
fn clamp_counter(val: i32, bits: usize) -> i8 {
    let max = (1i32 << (bits - 1)) - 1;
    let min = -(1i32 << (bits - 1));
    val.clamp(min, max) as i8
}

/// Statistical Corrector predictor.
///
/// GEHL counter tables are stored in a single flat `Vec<i8>` indexed as
/// `[table * table_size + entry]` to eliminate double indirection.
#[derive(Debug)]
pub struct StatCorrector {
    // --- GEHL tables ---
    /// Flattened GEHL counter tables: `num_tables * table_size` entries.
    counters: Vec<i8>,
    /// Number of active GEHL tables.
    num_tables: usize,
    /// History lengths for each GEHL table (fixed array).
    hist_lengths: [usize; MAX_SC_TABLES],
    /// Entries per GEHL table.
    table_size: usize,
    /// Number of bits for GEHL table indexing.
    table_bits: usize,
    /// Mask for indexing the GEHL tables.
    table_mask: usize,
    /// GEHL counter bit width.
    counter_bits: usize,

    // --- Bias tables (3 tables, Seznec CBP-5 design) ---
    /// Primary bias table: indexed by `PC ^ low_conf ^ pred_direction`.
    bias: Vec<i8>,
    /// Secondary bias table (SK): indexed by `PC ^ high_conf ^ pred_direction`.
    bias_sk: Vec<i8>,
    /// Bank bias table: indexed by composite hash of `PC`, bank, conf, alt, direction.
    bias_bank: Vec<i8>,
    /// Bias table mask.
    bias_mask: usize,
    /// Bias counter bit width.
    bias_counter_bits: usize,

    // --- Dynamic threshold (two-level: global + per-PC) ---
    /// Global threshold, stored `<<3` for sub-integer precision. Init = `initial_threshold << 3`.
    update_threshold: i32,
    /// Per-PC threshold adjustments (signed, -8..7 range by default).
    per_pc_threshold: Vec<i8>,
    /// Mask for per-PC threshold table indexing.
    per_pc_threshold_mask: usize,

    // --- Multi-tier chooser counters ---
    /// `FirstH` chooser: selects SC vs TAGE in the medium-confidence zone.
    first_h: i8,
    /// `SecondH` chooser: selects SC vs TAGE in the high-confidence narrow zone.
    second_h: i8,
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

        let bias_size = config.bias_table_size.next_power_of_two();
        let bias_mask = bias_size - 1;

        let per_pc_size = 1usize << config.per_pc_threshold_bits;
        let per_pc_mask = per_pc_size - 1;

        Self {
            counters: vec![0i8; config.num_tables * table_size],
            num_tables: config.num_tables,
            hist_lengths,
            table_size,
            table_bits,
            table_mask: table_size - 1,
            counter_bits: config.counter_bits,

            bias: vec![0i8; bias_size],
            bias_sk: vec![0i8; bias_size],
            bias_bank: vec![0i8; bias_size],
            bias_mask,
            bias_counter_bits: config.bias_counter_bits,

            update_threshold: (config.initial_threshold as i32) << 3,
            per_pc_threshold: vec![0i8; per_pc_size],
            per_pc_threshold_mask: per_pc_mask,

            first_h: -1,
            second_h: -1,
        }
    }

    /// Returns the counter value at `(table, idx)`.
    #[inline]
    fn counter(&self, table: usize, idx: usize) -> i8 {
        self.counters[table * self.table_size + idx]
    }

    /// Returns a mutable reference to the counter at `(table, idx)`.
    #[inline]
    fn counter_mut(&mut self, table: usize, idx: usize) -> &mut i8 {
        &mut self.counters[table * self.table_size + idx]
    }

    /// Computes the GEHL table index using word-level XOR-fold of GHR bits.
    #[inline]
    const fn table_index(&self, pc: u64, ghr: &Ghr, table: usize) -> usize {
        let pc_hash = (pc >> 2) as usize;
        let hl = self.hist_lengths[table];
        if hl == 0 {
            return pc_hash & self.table_mask;
        }

        let mask = if hl >= 64 { u64::MAX } else { (1u64 << hl) - 1 };
        let hist_bits = ghr.val() & mask;

        let mut h = hist_bits;
        let mut shift = self.table_bits;
        while shift < hl {
            h ^= hist_bits >> shift;
            shift += self.table_bits;
        }

        (pc_hash ^ h as usize) & self.table_mask
    }

    // --- Bias table indexing (Seznec CBP-5) ---

    #[inline]
    const fn bias_index(&self, pc: u64, meta: &TageScMeta) -> usize {
        let pc_hash = (pc >> 2) as usize;
        let low_conf = matches!(meta.conf, TageConfLevel::Low | TageConfLevel::None) as usize;
        let pred_inter = meta.pred_taken as usize;
        (pc_hash ^ (low_conf << 1) ^ pred_inter) & self.bias_mask
    }

    #[inline]
    fn bias_sk_index(&self, pc: u64, meta: &TageScMeta) -> usize {
        let pc_hash = (pc >> 2) as usize;
        let high_conf = (meta.conf == TageConfLevel::High) as usize;
        let pred_inter = meta.pred_taken as usize;
        (pc_hash ^ (high_conf << 1) ^ pred_inter) & self.bias_mask
    }

    #[inline]
    fn bias_bank_index(&self, pc: u64, meta: &TageScMeta) -> usize {
        let pc_hash = (pc >> 2) as usize;
        let high_conf = (meta.conf == TageConfLevel::High) as usize;
        let low_conf = matches!(meta.conf, TageConfLevel::Low | TageConfLevel::None) as usize;
        let alt_present = meta.alt_bank_present as usize;
        let pred_inter = meta.pred_taken as usize;
        let bank_bits = meta.provider_bank & 0xF;
        (pc_hash ^ bank_bits ^ (high_conf << 4) ^ (low_conf << 5) ^ (alt_present << 6) ^ pred_inter)
            & self.bias_mask
    }

    /// Computes the effective threshold for override decisions.
    #[inline]
    fn effective_threshold(&self, pc: u64) -> i32 {
        let base = self.update_threshold >> 3;
        let pc_adj = self.per_pc_threshold[(pc >> 2) as usize & self.per_pc_threshold_mask] as i32;
        base + pc_adj
    }

    /// Returns `true` if SC would override TAGE given the sum and meta.
    #[cfg(test)]
    pub fn would_override(
        &self,
        pc: u64,
        sc_taken: bool,
        meta: &TageScMeta,
        sc_sum: ScSum,
    ) -> bool {
        let thres = self.effective_threshold(pc);
        let sum_abs = sc_sum.0.abs();
        self.override_decision(sc_taken, meta, sum_abs, thres)
    }

    /// Multi-tier override decision. Returns `true` if SC should override TAGE.
    ///
    /// Implements Seznec's tiered confidence zones:
    /// - SC agrees with TAGE: no override
    /// - SC disagrees + High conf + |sum| < thres/4: revert to TAGE
    /// - SC disagrees + High conf + thres/4 <= |sum| < thres/2: `SecondH` chooser
    /// - SC disagrees + Medium conf + |sum| < thres/4: `FirstH` chooser
    /// - SC disagrees + |sum| < thres: revert to TAGE
    /// - SC disagrees + |sum| >= thres: use SC
    const fn override_decision(
        &self,
        sc_taken: bool,
        meta: &TageScMeta,
        sum_abs: i32,
        thres: i32,
    ) -> bool {
        // SC agrees with TAGE — no override.
        if sc_taken == meta.pred_taken {
            return false;
        }

        // SC disagrees — multi-tier check.
        if sum_abs >= thres {
            return true;
        }

        let quarter = thres / 4;
        let half = thres / 2;

        match meta.conf {
            TageConfLevel::High => {
                if sum_abs < quarter {
                    false
                } else if sum_abs < half {
                    // SecondH chooser zone
                    self.second_h >= 0
                } else {
                    false
                }
            }
            TageConfLevel::Medium => {
                if sum_abs < quarter {
                    // FirstH chooser zone
                    self.first_h >= 0
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Computes the SC prediction. Returns `(corrected_prediction, ScSum)`.
    ///
    /// Sum = weighted centered TAGE counter (seed) + centered bias counters
    ///       (3 tables) + centered GEHL counters.
    /// Seeding with the TAGE counter ensures SC must overcome TAGE confidence
    /// to override. Weight of 3 matches the number of bias tables, ensuring
    /// the TAGE term provides meaningful pushback against bias-driven overrides.
    pub fn predict(&self, pc: u64, ghr: &Ghr, meta: &TageScMeta) -> (bool, ScSum) {
        // Seed with TAGE provider confidence (centered counter).
        let mut sum: i32 = 2 * (meta.pred_ctr as i32) + 1;

        // Bias tables: centered as 2*ctr+1.
        let bi = self.bias_index(pc, meta);
        sum += 2 * (self.bias[bi] as i32) + 1;

        let bsi = self.bias_sk_index(pc, meta);
        sum += 2 * (self.bias_sk[bsi] as i32) + 1;

        let bbi = self.bias_bank_index(pc, meta);
        sum += 2 * (self.bias_bank[bbi] as i32) + 1;

        // GEHL tables: centered as 2*ctr+1.
        for t in 0..self.num_tables {
            let idx = self.table_index(pc, ghr, t);
            sum += 2 * (self.counter(t, idx) as i32) + 1;
        }

        let sc_taken = sum >= 0;
        let thres = self.effective_threshold(pc);

        let corrected = if self.override_decision(sc_taken, meta, sum.abs(), thres) {
            sc_taken
        } else {
            meta.pred_taken
        };

        (corrected, ScSum(sum))
    }

    /// Updates the SC tables at commit time.
    pub fn update(&mut self, pc: u64, ghr: &Ghr, taken: bool, meta: &TageScMeta, sc_sum: ScSum) {
        let sum = sc_sum.0;
        let sc_taken = sum >= 0;
        let thres = self.effective_threshold(pc);
        let sum_abs = sum.abs();

        let quarter = thres / 4;
        let half = thres / 2;

        // Update FirstH/SecondH choosers when in their respective zones.
        if sc_taken != meta.pred_taken {
            // SecondH zone: High conf + thres/4 <= |sum| < thres/2
            if meta.conf == TageConfLevel::High && sum_abs >= quarter && sum_abs < half {
                if sc_taken == taken {
                    self.second_h = (self.second_h + 1).min(63);
                } else {
                    self.second_h = (self.second_h - 1).max(-64);
                }
            }
            // FirstH zone: Medium conf + |sum| < thres/4
            if meta.conf == TageConfLevel::Medium && sum_abs < quarter {
                if sc_taken == taken {
                    self.first_h = (self.first_h + 1).min(63);
                } else {
                    self.first_h = (self.first_h - 1).max(-64);
                }
            }
        }

        // Threshold adaptation: only when SC disagrees with TAGE.
        // SC disagrees and would have been right -> lower threshold (encourage overrides).
        // SC disagrees and would have been wrong -> raise threshold (discourage overrides).
        // SC agrees with TAGE -> no threshold change.
        if sc_taken != meta.pred_taken {
            let pc_idx = (pc >> 2) as usize & self.per_pc_threshold_mask;
            if sc_taken == taken {
                // SC would have been right to override -> lower threshold.
                self.update_threshold = (self.update_threshold - 1).max(0);
                self.per_pc_threshold[pc_idx] = (self.per_pc_threshold[pc_idx] - 1).max(-16);
            } else {
                // SC would have been wrong to override -> raise threshold.
                self.update_threshold = (self.update_threshold + 1).min(511);
                self.per_pc_threshold[pc_idx] = (self.per_pc_threshold[pc_idx] + 1).min(15);
            }
        }

        // Training condition (Seznec's exact): sc_pred != taken || |sum| < threshold.
        let sc_pred = if self.override_decision(sc_taken, meta, sum_abs, thres) {
            sc_taken
        } else {
            meta.pred_taken
        };

        let should_train = sc_pred != taken || sum_abs < thres;
        if !should_train {
            return;
        }

        // Update all 3 bias tables (bias_counter_bits clamp).
        let bbits = self.bias_counter_bits;
        let bi = self.bias_index(pc, meta);
        let v = self.bias[bi] as i32;
        self.bias[bi] =
            if taken { clamp_counter(v + 1, bbits) } else { clamp_counter(v - 1, bbits) };

        let bsi = self.bias_sk_index(pc, meta);
        let v = self.bias_sk[bsi] as i32;
        self.bias_sk[bsi] =
            if taken { clamp_counter(v + 1, bbits) } else { clamp_counter(v - 1, bbits) };

        let bbi = self.bias_bank_index(pc, meta);
        let v = self.bias_bank[bbi] as i32;
        self.bias_bank[bbi] =
            if taken { clamp_counter(v + 1, bbits) } else { clamp_counter(v - 1, bbits) };

        // Update all GEHL tables (counter_bits clamp).
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
            bias_table_size: 256,
            bias_counter_bits: 6,
            initial_threshold: 35,
            per_pc_threshold_bits: 6,
        }
    }

    fn default_meta(pred_taken: bool) -> TageScMeta {
        TageScMeta {
            conf: TageConfLevel::Low,
            provider_bank: 0,
            alt_bank_present: false,
            pred_taken,
            pred_ctr: if pred_taken { 0 } else { -1 },
        }
    }

    #[test]
    fn test_sc_initial_prediction_follows_base() {
        let sc = StatCorrector::new(&test_config());
        let ghr = Ghr::with_len(64);
        let meta = default_meta(true);

        // With all-zero counters, each centered counter contributes 2*0+1 = 1.
        // Total = 3 bias + 4 GEHL = 7, which is positive (sc_taken = true)
        // but well below threshold ~35, so SC should NOT override.
        let (pred, _sum) = sc.predict(0x1000, &ghr, &meta);
        assert!(pred, "SC should follow TAGE when counters are zero (sum << threshold)");
    }

    #[test]
    fn test_sc_untrained_never_overrides() {
        let sc = StatCorrector::new(&test_config());
        let ghr = Ghr::with_len(64);

        // Even when TAGE predicts not-taken, untrained SC should not override.
        // Sum = 7 (all +1 centered), |7| < 35 threshold, so no override.
        let meta = default_meta(false);
        let (pred, sum) = sc.predict(0x1000, &ghr, &meta);
        assert!(!pred, "SC should follow TAGE not-taken when untrained");
        // Sum should be positive (7) but below threshold.
        assert!(sum.0 > 0, "Sum should be positive from centered counters");
        assert!(sum.0 < 35, "Sum should be well below threshold");
    }

    #[test]
    fn test_sc_can_correct() {
        let config = test_config();
        let mut sc = StatCorrector::new(&config);
        let ghr = Ghr::with_len(64);
        let pc = 0x1000u64;
        let meta = default_meta(true);

        // Train SC heavily: branch is always not-taken but TAGE predicts taken.
        for _ in 0..200 {
            let (_pred, sum) = sc.predict(pc, &ghr, &meta);
            sc.update(pc, &ghr, false, &meta, sum);
        }

        let (pred, _sum) = sc.predict(pc, &ghr, &meta);
        assert!(!pred, "SC should correct weak base prediction after heavy training");
    }

    #[test]
    fn test_sc_threshold_stability() {
        // Regression test: threshold must not drain toward zero when TAGE is
        // mostly correct. Run 1000 branches where TAGE is correct ~90% of the
        // time. Threshold should stay above initial/2.
        let config = test_config();
        let initial_threshold = (config.initial_threshold as i32) << 3;
        let mut sc = StatCorrector::new(&config);
        let ghr = Ghr::with_len(64);

        for i in 0u64..1000 {
            let pc = 0x1000 + (i % 16) * 4;
            // TAGE predicts taken; branch is taken ~90% of the time.
            let taken = i % 10 != 0;
            let meta = default_meta(true);
            let (_pred, sum) = sc.predict(pc, &ghr, &meta);
            sc.update(pc, &ghr, taken, &meta, sum);
        }

        assert!(
            sc.update_threshold >= initial_threshold / 2,
            "Threshold drained to {} (initial was {}); adaptation is too aggressive",
            sc.update_threshold,
            initial_threshold,
        );
    }
}

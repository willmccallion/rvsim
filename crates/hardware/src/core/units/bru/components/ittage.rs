//! Indirect Target TAGE (ITTAGE) predictor.
//!
//! A TAGE-like predictor for indirect branch **targets** (not direction).
//! Based on Seznec's ITTAGE (MICRO 2011). Uses tagged tables indexed by
//! geometric history lengths, but stores target addresses instead of
//! direction counters.
//!
//! ITTAGE does not own its own GHR — the parent predictor passes the GHR
//! into speculate/repair calls, avoiding redundant 128-byte copies.

use super::tagged_bank::GeoBankSet;
use crate::config::IttageConfig;
use crate::core::units::bru::Ghr;

/// An entry in an ITTAGE bank.
#[derive(Clone, Copy, Debug, Default)]
struct IttageEntry {
    /// Tag for matching the history/PC hash.
    tag: u16,
    /// Predicted target address.
    target: u64,
    /// 2-bit useful counter for replacement policy.
    u: u8,
}

/// Indirect Target TAGE predictor.
///
/// Predicts targets for indirect branches (JALR non-return) using
/// geometric-history tagged tables. Each entry stores a target address
/// rather than a direction counter.
///
/// Does **not** own a GHR — the caller provides GHR references for
/// speculate/repair/update to avoid redundant copies.
#[derive(Debug)]
pub struct Ittage {
    /// Shared CSR/indexing infrastructure (owns its own CSRs for its history lengths).
    banks: GeoBankSet,
    /// Tagged tables (one per bank). Heap-allocated — entries are large.
    tables: Vec<Vec<IttageEntry>>,
    /// Counter for periodic reset of useful bits.
    clock_counter: u32,
    /// Interval for resetting useful bits.
    reset_interval: u32,
}

impl Ittage {
    /// Creates a new ITTAGE predictor from config.
    ///
    /// # Panics
    ///
    /// Panics if `history_lengths` or `tag_widths` length does not match `num_banks`.
    pub fn new(config: &IttageConfig) -> Self {
        let num_banks = config.num_banks;
        let table_size = config.table_size.next_power_of_two();
        let table_bits = table_size.trailing_zeros() as usize;

        assert_eq!(
            config.history_lengths.len(),
            num_banks,
            "ITTAGE: history_lengths.len() ({}) must match num_banks ({num_banks})",
            config.history_lengths.len(),
        );
        assert_eq!(
            config.tag_widths.len(),
            num_banks,
            "ITTAGE: tag_widths.len() ({}) must match num_banks ({num_banks})",
            config.tag_widths.len(),
        );

        let banks = GeoBankSet::new(&config.history_lengths, &config.tag_widths, table_bits);

        let mut tables = Vec::with_capacity(num_banks);
        for _ in 0..num_banks {
            tables.push(vec![IttageEntry::default(); table_size]);
        }

        Self { banks, tables, clock_counter: 0, reset_interval: config.reset_interval }
    }

    /// Predicts the target for an indirect branch. Returns `Some(target)` on hit.
    pub fn predict(&self, pc: u64) -> Option<u64> {
        for i in (0..self.banks.num_banks()).rev() {
            let idx = self.banks.spec_index(pc, i);
            let tag = self.banks.spec_tag(pc, i);
            let entry = &self.tables[i][idx];
            if entry.tag == tag && entry.target != 0 {
                return Some(entry.target);
            }
        }
        None
    }

    /// Updates ITTAGE at commit time with the actual target.
    pub fn update(&mut self, pc: u64, target: u64, ghr_snapshot: &Ghr) {
        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for table in &mut self.tables {
                for entry in table {
                    entry.u >>= 1;
                }
            }
        }

        let num_banks = self.banks.num_banks();

        // Find provider (longest matching bank).
        let mut provider = None;
        for i in (0..num_banks).rev() {
            let idx = self.banks.snapshot_index(pc, i, ghr_snapshot);
            let tag = self.banks.snapshot_tag(pc, i, ghr_snapshot);
            if self.tables[i][idx].tag == tag {
                provider = Some(i);
                break;
            }
        }

        if let Some(bank) = provider {
            let idx = self.banks.snapshot_index(pc, bank, ghr_snapshot);
            let entry = &mut self.tables[bank][idx];

            if entry.target == target {
                entry.u = entry.u.saturating_add(1).min(3);
            } else if entry.u > 0 {
                entry.u -= 1;
            } else {
                entry.target = target;
            }
        }

        // On misprediction, try to allocate in a longer-history bank.
        let mispredicted = !provider.is_some_and(|bank| {
            let idx = self.banks.snapshot_index(pc, bank, ghr_snapshot);
            self.tables[bank][idx].target == target
        });

        if mispredicted {
            let start = provider.map_or(0, |b| b + 1);
            if start < num_banks {
                let mut allocated = false;
                for i in start..num_banks {
                    let idx = self.banks.snapshot_index(pc, i, ghr_snapshot);
                    let tag = self.banks.snapshot_tag(pc, i, ghr_snapshot);
                    let entry = &mut self.tables[i][idx];

                    if entry.u == 0 {
                        entry.tag = tag;
                        entry.target = target;
                        entry.u = 1;
                        allocated = true;
                        break;
                    }
                }

                if !allocated {
                    for i in start..num_banks {
                        let idx = self.banks.snapshot_index(pc, i, ghr_snapshot);
                        if self.tables[i][idx].u > 0 {
                            self.tables[i][idx].u -= 1;
                        }
                    }
                }
            }
        }
    }

    /// Speculatively updates ITTAGE's CSRs for a new branch outcome.
    /// Must be called BEFORE the caller's `ghr.push()`.
    #[inline]
    pub fn speculate(&mut self, taken: bool, ghr: &Ghr) {
        self.banks.update_csrs(taken, ghr);
    }

    /// Recomputes ITTAGE's CSRs from a snapshot GHR.
    pub fn repair_history(&mut self, ghr: &Ghr) {
        self.banks.recompute_all(ghr);
    }

    /// Recomputes ITTAGE's CSRs from the committed GHR.
    pub fn repair_to_committed(&mut self, committed_ghr: &Ghr) {
        self.banks.recompute_all(committed_ghr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> IttageConfig {
        IttageConfig {
            num_banks: 4,
            table_size: 64,
            history_lengths: vec![4, 8, 16, 32],
            tag_widths: vec![9, 9, 10, 10],
            reset_interval: 100_000,
        }
    }

    #[test]
    fn test_ittage_miss_then_hit() {
        let mut ittage = Ittage::new(&test_config());
        let pc = 0x8000_2000u64;
        let target = 0x8000_3000u64;
        let mut ghr = Ghr::with_len(32);

        // Initially no prediction (default entries have target=0, filtered out).
        assert_eq!(ittage.predict(pc), None);

        // Train with a target.
        ittage.update(pc, target, &ghr);

        // After training, should predict the target.
        assert_eq!(ittage.predict(pc), Some(target));

        // Speculate should not crash.
        ittage.speculate(true, &ghr);
        ghr.push(true);
    }

    #[test]
    fn test_ittage_speculate_and_repair() {
        let mut ittage = Ittage::new(&test_config());
        let pc = 0x8000_1000u64;
        let target = 0x8000_5000u64;
        let mut ghr = Ghr::with_len(32);

        // Push some speculative history.
        for _ in 0..10 {
            ittage.speculate(true, &ghr);
            ghr.push(true);
        }
        let snapshot = ghr;

        // Push more (wrong path).
        for _ in 0..5 {
            ittage.speculate(false, &ghr);
            ghr.push(false);
        }

        // Repair to snapshot.
        ghr = snapshot;
        ittage.repair_history(&ghr);

        // Train and verify prediction works after repair.
        ittage.update(pc, target, &ghr);
        assert_eq!(ittage.predict(pc), Some(target));
    }
}

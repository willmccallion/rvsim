//! TAGE (Tagged Geometric History Length) Branch Predictor.
//!
//! TAGE uses a base bimodal predictor and multiple tagged banks indexed with
//! geometrically increasing history lengths. It provides high accuracy by
//! matching long history patterns while falling back to shorter histories
//! or the base predictor when necessary.
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `predict()`: O(B) where B is the number of banks (typically 4-6)
//!   - `update()`: O(B)
//! - **Space Complexity:** O(T × B) where T is table size per bank
//! - **Hardware Cost:** High - multiple table lookups, priority selection
//! - **Best Case:** Complex history-correlated patterns with varying lengths
//! - **Worst Case:** Random or completely uncorrelated branches (~50% accuracy)

use super::{BranchPredictor, btb::Btb, ras::Ras};
use crate::config::TageConfig;

/// An entry in a TAGE bank.
#[derive(Clone, Default)]
struct TageEntry {
    /// Tag for matching the history/PC hash.
    tag: u16,
    /// 3-bit saturating counter for prediction.
    ctr: i8,
    /// 2-bit useful counter for replacement policy.
    u: u8,
}

/// Loop Predictor Entry for handling loop exit branches.
#[derive(Clone, Default)]
struct LoopEntry {
    /// Tag for matching the branch PC.
    tag: u16,
    /// Confidence counter.
    conf: u8,
    /// Current iteration count.
    count: u16,
    /// Iteration limit detected.
    limit: u16,
    /// Age/Usefulness counter.
    age: u8,
    /// Predicted direction.
    dir: bool,
}

/// TAGE Predictor structure.
pub struct TagePredictor {
    /// Branch Target Buffer.
    btb: Btb,
    /// Return Address Stack.
    ras: Ras,
    /// Global History Register.
    ghr: u64,
    /// Path History Register.
    phr: u64,

    /// Base bimodal predictor table.
    base: Vec<i8>,
    /// Tagged component banks.
    banks: Vec<Vec<TageEntry>>,

    /// Geometric history lengths for each bank.
    hist_lengths: Vec<usize>,
    /// Tag widths for each bank.
    tag_widths: Vec<usize>,
    /// Mask for indexing the tables.
    table_mask: usize,

    /// Loop predictor table.
    loops: Vec<LoopEntry>,
    /// Mask for indexing the loop table.
    loop_mask: usize,

    /// Index of the bank providing the current prediction.
    provider_bank: usize,
    /// Index of the alternative bank.
    alt_bank: usize,

    /// Counter for periodic reset of useful bits.
    clock_counter: u32,
    /// Interval for resetting useful bits.
    reset_interval: u32,
}

impl TagePredictor {
    /// Creates a new TAGE Predictor based on configuration.
    pub fn new(config: &TageConfig, btb_size: usize, ras_size: usize) -> Self {
        assert!(
            config.table_size.is_power_of_two(),
            "TAGE table size must be power of 2"
        );
        assert!(
            config.loop_table_size.is_power_of_two(),
            "TAGE loop table size must be power of 2"
        );

        let (hist_lengths, tag_widths, num_banks) = if !config.history_lengths.is_empty() {
            (
                config.history_lengths.clone(),
                config.tag_widths.clone(),
                config.num_banks,
            )
        } else {
            (vec![5, 15, 44, 130], vec![9, 9, 10, 10], 4)
        };

        assert_eq!(
            hist_lengths.len(),
            num_banks,
            "TAGE: History lengths vector must match num_banks"
        );
        assert_eq!(
            tag_widths.len(),
            num_banks,
            "TAGE: Tag widths vector must match num_banks"
        );

        let mut banks = Vec::new();
        for _ in 0..num_banks {
            banks.push(vec![TageEntry::default(); config.table_size]);
        }

        Self {
            btb: Btb::new(btb_size),
            ras: Ras::new(ras_size),
            ghr: 0,
            phr: 0,
            base: vec![0; config.table_size],
            banks,
            hist_lengths,
            tag_widths,
            table_mask: config.table_size - 1,

            loops: vec![LoopEntry::default(); config.loop_table_size],
            loop_mask: config.loop_table_size - 1,

            provider_bank: 0,
            alt_bank: 0,
            clock_counter: 0,
            reset_interval: config.reset_interval,
        }
    }

    /// Calculates the index for a specific bank using PC, GHR, and PHR.
    fn index(&self, pc: u64, bank: usize) -> usize {
        let len = self.hist_lengths[bank];
        let mask = if len >= 64 {
            u64::MAX
        } else {
            (1u64 << len) - 1
        };

        let h = self.ghr & mask;
        let ph = self.phr & mask;
        ((pc ^ h ^ (ph << 1)) as usize) & self.table_mask
    }

    /// Calculates the tag for a specific bank using PC and GHR.
    fn tag(&self, pc: u64, bank: usize) -> u16 {
        let len = self.hist_lengths[bank];
        let width = self.tag_widths[bank];

        let mask = if len >= 64 {
            u64::MAX
        } else {
            (1u64 << len) - 1
        };

        let h = self.ghr & mask;
        let tag = pc ^ (h >> 3);
        (tag as u16) & ((1 << width) - 1)
    }

    /// Checks the loop predictor for a matching entry.
    fn get_loop_pred(&self, pc: u64) -> Option<bool> {
        let idx = (pc as usize) & self.loop_mask;
        let e = &self.loops[idx];
        let tag = ((pc >> 8) & 0xFFFF) as u16;

        if e.tag == tag && e.conf == 3 {
            if e.count < e.limit {
                return Some(e.dir);
            } else {
                return Some(!e.dir);
            }
        }
        None
    }
}

impl BranchPredictor for TagePredictor {
    /// Predicts branch direction and target.
    ///
    /// Checks the loop predictor first, then searches the tagged banks for the
    /// longest history match (provider). If no match is found, uses the base predictor.
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        if let Some(loop_pred) = self.get_loop_pred(pc) {
            return (loop_pred, self.btb.lookup(pc));
        }

        let mut provider = 0;
        let num_banks = self.banks.len();

        for i in (0..num_banks).rev() {
            let idx = self.index(pc, i);
            let tag = self.tag(pc, i);
            if self.banks[i][idx].tag == tag {
                provider = i + 1;
                break;
            }
        }

        if provider > 0 {
            let bank_idx = provider - 1;
            let idx = self.index(pc, bank_idx);
            let ctr = self.banks[bank_idx][idx].ctr;
            return (ctr >= 0, self.btb.lookup(pc));
        }

        let base_idx = (pc as usize) & self.table_mask;
        (self.base[base_idx] >= 0, self.btb.lookup(pc))
    }

    /// Updates the predictor state.
    ///
    /// Updates the loop predictor, the provider bank, and potentially allocates
    /// a new entry in a bank with longer history on mispredictions. Also handles
    /// periodic resetting of useful bits.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>) {
        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for bank in &mut self.banks {
                for entry in bank {
                    entry.u >>= 1;
                }
            }
        }

        let mut provider = 0;
        let mut alt = 0;
        let num_banks = self.banks.len();

        for i in (0..num_banks).rev() {
            let idx = self.index(pc, i);
            let tag = self.tag(pc, i);
            if self.banks[i][idx].tag == tag {
                if provider == 0 {
                    provider = i + 1;
                } else if alt == 0 {
                    alt = i + 1;
                    break;
                }
            }
        }
        self.provider_bank = provider;
        self.alt_bank = alt;

        let pred_taken = if self.provider_bank > 0 {
            let idx = self.index(pc, self.provider_bank - 1);
            self.banks[self.provider_bank - 1][idx].ctr >= 0
        } else {
            let base_idx = (pc as usize) & self.table_mask;
            self.base[base_idx] >= 0
        };

        let alt_taken = if self.alt_bank > 0 {
            let idx = self.index(pc, self.alt_bank - 1);
            self.banks[self.alt_bank - 1][idx].ctr >= 0
        } else {
            let base_idx = (pc as usize) & self.table_mask;
            self.base[base_idx] >= 0
        };

        let mispredicted = pred_taken != taken;

        let l_idx = (pc as usize) & self.loop_mask;
        let l_tag = ((pc >> 8) & 0xFFFF) as u16;
        let loop_entry = &mut self.loops[l_idx];

        if loop_entry.tag == l_tag {
            if loop_entry.age < 255 {
                loop_entry.age += 1;
            }

            if taken == loop_entry.dir {
                loop_entry.count += 1;
            } else {
                if loop_entry.count == loop_entry.limit {
                    if loop_entry.conf < 3 {
                        loop_entry.conf += 1;
                    }
                } else {
                    loop_entry.limit = loop_entry.count;
                    loop_entry.conf = 0;
                    loop_entry.age = 0;
                }
                loop_entry.count = 0;
            }
        } else if loop_entry.age == 0 {
            loop_entry.tag = l_tag;
            loop_entry.limit = 0;
            loop_entry.count = 0;
            loop_entry.conf = 0;
            loop_entry.age = 255;
            loop_entry.dir = taken;
        } else {
            loop_entry.age -= 1;
        }

        if self.provider_bank > 0 {
            let bank_idx = self.provider_bank - 1;
            let idx = self.index(pc, bank_idx);
            let e = &mut self.banks[bank_idx][idx];

            if taken {
                if e.ctr < 3 {
                    e.ctr += 1;
                }
            } else if e.ctr > -4 {
                e.ctr -= 1;
            }

            if !mispredicted && (alt_taken != taken) && e.u < 3 {
                e.u += 1;
            }
        } else {
            let base_idx = (pc as usize) & self.table_mask;
            let b = &mut self.base[base_idx];
            if taken {
                if *b < 1 {
                    *b += 1;
                }
            } else if *b > -2 {
                *b -= 1;
            }
        }

        if mispredicted {
            let start_bank = if self.provider_bank == 0 {
                0
            } else {
                self.provider_bank
            };

            if start_bank < num_banks {
                let mut allocated = false;
                for i in start_bank..num_banks {
                    let idx = self.index(pc, i);
                    let tag = self.tag(pc, i);
                    let e = &mut self.banks[i][idx];

                    if e.u == 0 {
                        e.tag = tag;
                        e.ctr = if taken { 0 } else { -1 };
                        e.u = 1;
                        allocated = true;
                        break;
                    }
                }

                if !allocated {
                    for i in start_bank..num_banks {
                        let idx = self.index(pc, i);
                        if self.banks[i][idx].u > 0 {
                            self.banks[i][idx].u -= 1;
                        }
                    }
                }
            }
        }

        self.ghr = (self.ghr << 1) | (if taken { 1 } else { 0 });
        self.phr = (self.phr << 1) | (pc & 1);

        if let Some(tgt) = target {
            self.btb.update(pc, tgt);
        }
    }

    /// Predicts the target of a jump instruction using the BTB.
    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb.lookup(pc)
    }

    /// Handles a function call by pushing the return address to the RAS.
    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        self.ras.push(ret_addr);
        self.btb.update(pc, target);
    }

    /// Predicts the return address using the RAS.
    fn predict_return(&self) -> Option<u64> {
        self.ras.top()
    }

    /// Handles a function return by popping from the RAS.
    fn on_return(&mut self) {
        self.ras.pop();
    }
}

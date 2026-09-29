//! TAGE core tests.

use super::*;

fn test_config() -> TageConfig {
    TageConfig {
        num_banks: 4,
        table_size: 256,
        reset_interval: 100_000,
        history_lengths: vec![5, 15, 44, 130],
        tag_widths: vec![9, 9, 10, 10],
        ..TageConfig::default()
    }
}

#[test]
fn a_branch_never_seen_is_predicted_not_taken() {
    let tage = TageCore::new(&test_config());

    // Its tags are not 0, which every empty tagged entry holds.
    let prediction = tage.predict(0x8000_1004);

    assert!(!prediction.taken());
}

#[test]
fn restoring_a_checkpoint_restores_the_prediction() {
    let mut tage = TageCore::new(&test_config());
    let pc = 0x8000_1234u64;
    for i in 0u64..50 {
        tage.speculate(0x8000_0000, i % 3 != 0, HistoryBranch::Conditional);
    }
    let checkpoint = tage.checkpoint();
    let saved = tage.predict(pc);

    for _ in 0..30 {
        tage.speculate(0x8000_0000, true, HistoryBranch::Conditional);
    }
    tage.restore(&checkpoint);

    assert_eq!(tage.predict(pc).indices, saved.indices);
}

#[test]
fn a_branch_trains_the_entries_its_prediction_read() {
    let mut tage = TageCore::new(&test_config());
    for i in 0u64..40 {
        tage.speculate(0x8000_0000 + 4 * i, i % 3 == 0, HistoryBranch::Conditional);
    }
    let at_prediction = tage.checkpoint();
    let pc = 0x8000_2040u64;

    for _ in 0..30 {
        tage.restore(&at_prediction);
        let prediction = tage.predict(pc);
        for _ in 0..8 {
            tage.speculate(0x8000_0000, true, HistoryBranch::Conditional);
        }
        tage.update(false, &prediction, prediction.taken());
    }
    tage.restore(&at_prediction);

    assert!(!tage.predict(pc).taken(), "trained under the history it predicted with");
}

#[test]
fn the_path_fold_matches_tagebase_f() {
    assert_eq!(fold_path(0xABCD, 16, 1, 11), 0x7CE);
    assert_eq!(fold_path(0xABCD, 16, 2, 11), 0x665);
    assert_eq!(fold_path(0xABCD, 5, 1, 11), fold_path(0xD, 5, 1, 11));
}

#[test]
fn a_wide_path_folds_as_tagebase_f_does() {
    assert_eq!(fold_path(0x5AB_CDEF, 27, 3, 10), 0x1F);
    assert_eq!(fold_path(0x5AB_CDEF, 27, 12, 10), 0x16B1C);
}

#[test]
fn the_path_history_separates_branches_with_the_same_global_history() {
    let mut tage = TageCore::new(&test_config());
    let pc = 0x8000_2040u64;

    tage.histories.set_path(0b1010);
    let one_path = tage.predict(pc);
    tage.histories.set_path(0b0101);
    let other_path = tage.predict(pc);

    assert_ne!(one_path.indices, other_path.indices);
}

/// A prediction that read entry `bank * 16` in every bank, with the
/// given provider, alternate and predictions.
fn prediction(
    provider: Option<usize>,
    alt: Option<usize>,
    provider_taken: bool,
    alt_taken: bool,
    taken: bool,
) -> TagePrediction {
    let mut indices = [0usize; MAX_BANKS];
    let mut tags = [0u16; MAX_BANKS];
    for bank in 0..MAX_BANKS {
        indices[bank] = bank * 16;
        tags[bank] = 0x55 + bank as u16;
    }
    TagePrediction {
        indices,
        tags,
        base_index: 0,
        provider,
        alt,
        provider_taken,
        alt_taken,
        provider_weak: true,
        use_alt_index: 0,
        meta: TageScMeta {
            conf: TageConfLevel::None,
            provider_bank: provider.map_or(0, |bank| bank + 1),
            alt_bank_present: alt.is_some(),
            pred_taken: taken,
            provider_disagrees_with_alt: provider_taken != alt_taken,
        },
    }
}

fn allocated_banks(tage: &TageCore, prediction: &TagePrediction) -> Vec<usize> {
    (0..tage.num_banks)
        .filter(|&bank| {
            tage.storage[tage.bank_storage[bank]][prediction.indices[bank]].tag
                == prediction.tags[bank]
        })
        .collect()
}

#[test]
fn a_mispredict_with_no_free_entry_frees_one_of_the_next_three_tables() {
    let mut tage = TageCore::new(&test_config());
    let wrong = prediction(None, None, true, true, true);
    for bank in 0..4 {
        tage.storage[tage.bank_storage[bank]][wrong.indices[bank]].u = 3;
    }

    tage.update(false, &wrong, wrong.taken());

    let taken = allocated_banks(&tage, &wrong);
    assert_eq!(taken.len(), 1, "one entry allocated");
    assert!(taken[0] <= 2, "among the next three tables, got {}", taken[0]);
}

#[test]
fn a_weak_new_entry_that_was_right_allocates_nothing() {
    let mut tage = TageCore::new(&test_config());
    // The alternate overrode a new provider that was right.
    let overridden = prediction(Some(0), None, false, true, true);

    tage.update(false, &overridden, overridden.taken());

    assert!(allocated_banks(&tage, &overridden).is_empty());
}

#[test]
fn the_alternate_trains_while_the_provider_has_not_proved_useful() {
    let mut tage = TageCore::new(&test_config());
    let p = prediction(Some(1), Some(0), true, true, true);

    tage.update(false, &p, p.taken());

    assert_eq!(
        tage.storage[tage.bank_storage[0]][p.indices[0]].ctr, -1,
        "the alternate moved toward not taken"
    );
}

#[test]
fn test_update_trains_predictor() {
    let config = test_config();
    let mut tage = TageCore::new(&config);
    let pc = 0x8000_1000u64;

    // Train not-taken heavily.
    for _ in 0..50 {
        let prediction = tage.predict(pc);
        tage.update(false, &prediction, prediction.taken());
    }

    assert!(!tage.predict(pc).taken(), "Should predict not-taken after heavy training");
}

#[test]
fn several_use_alt_counters_split_by_bank_group_and_alternate_confidence() {
    let tage = TageCore::new(&TageConfig {
        num_banks: 12,
        history_lengths: (1..=12).map(|i| i * 10).collect(),
        tag_widths: vec![10; 12],
        use_alt_counters: 16,
        use_alt_bits: 5,
        ..TageConfig::default()
    });

    let picks = [
        tage.use_alt_index(Some(0), false),
        tage.use_alt_index(Some(7), true),
        tage.use_alt_index(Some(8), false),
        tage.use_alt_index(Some(11), true),
    ];

    assert_eq!(picks, [0, 1, 2, 3]);
}

#[test]
fn one_use_alt_counter_serves_every_prediction() {
    let tage = TageCore::new(&test_config());

    assert_eq!(tage.use_alt_index(Some(3), true), 0);
}

fn cbp5_config() -> TageConfig {
    TageConfig {
        num_banks: 8,
        table_size: 256,
        history_lengths: vec![4, 6, 10, 16, 25, 40, 64, 100],
        tag_widths: vec![9; 8],
        useful_bits: 1,
        max_allocations: 2,
        allocation: TageAllocation::Cbp5,
        update: TageUpdate::Cbp5,
        ..TageConfig::default()
    }
}

#[test]
fn cbp5_allocates_up_to_max_allocations_skipping_a_pair_after_each() {
    let mut tage = TageCore::new(&cbp5_config());
    let wrong = prediction(None, None, true, true, true);

    tage.update(false, &wrong, true);

    let taken = allocated_banks(&tage, &wrong);
    assert_eq!(taken.len(), 2, "allocated {taken:?}");
    assert!(taken[1] - taken[0] >= 3, "a pair skipped between {taken:?}");
}

#[test]
fn cbp5_allocates_rarely_when_the_final_prediction_was_right() {
    let mut tage = TageCore::new(&cbp5_config());
    let tage_wrong = prediction(None, None, true, true, true);
    let mut allocations = 0;

    for _ in 0..320 {
        tage.update(false, &tage_wrong, false);
        for bank in allocated_banks(&tage, &tage_wrong) {
            tage.storage[tage.bank_storage[bank]][tage_wrong.indices[bank]].tag = 0;
            allocations += 1;
        }
    }

    assert!((1..=40).contains(&allocations), "{allocations} allocations in 320");
}

#[test]
fn cbp5_decays_a_strong_unuseful_entry_instead_of_replacing_it() {
    let mut tage = TageCore::new(&cbp5_config());
    let wrong = prediction(None, None, true, true, true);
    for bank in 0..8 {
        tage.storage[tage.bank_storage[bank]][wrong.indices[bank]].ctr = 3;
    }

    tage.update(false, &wrong, true);

    assert!(allocated_banks(&tage, &wrong).is_empty());
    let ctrs: Vec<i8> =
        (0..8).map(|bank| tage.storage[tage.bank_storage[bank]][wrong.indices[bank]].ctr).collect();
    assert!(ctrs.contains(&2) && ctrs.iter().all(|&ctr| ctr >= 2), "{ctrs:?}");
}

#[test]
fn cbp5_a_provider_turning_weak_loses_its_usefulness() {
    let mut tage = TageCore::new(&cbp5_config());
    let p = prediction(Some(1), Some(0), true, true, true);
    let provider = &mut tage.storage[tage.bank_storage[1]][p.indices[1]];
    provider.ctr = 1;
    provider.u = 1;

    tage.update(false, &p, true);

    assert_eq!(tage.storage[tage.bank_storage[1]][p.indices[1]].u, 0);
}

#[test]
fn cbp5_a_strong_provider_that_was_wrong_leaves_the_alternate_alone() {
    let mut tage = TageCore::new(&cbp5_config());
    let p = prediction(Some(1), Some(0), true, true, true);
    tage.storage[tage.bank_storage[1]][p.indices[1]].ctr = 3;

    tage.update(false, &p, true);

    assert_eq!(tage.storage[tage.bank_storage[0]][p.indices[0]].ctr, 0);
}

#[test]
fn neighbouring_bimodal_entries_share_a_hysteresis_bit() {
    let mut bimodal = Bimodal::new(16, 2);
    bimodal.train(0, true);
    bimodal.train(0, true);
    let neighbour_before = bimodal.counter(1);

    bimodal.train(0, false);

    assert_eq!((neighbour_before, bimodal.counter(1)), (-1, -2));
    assert_eq!(bimodal.counter(4), -1, "entry 4 has its own hysteresis bit");
}

fn banked_config() -> TageConfig {
    TageConfig {
        num_banks: 6,
        table_size: 64,
        history_lengths: vec![4, 4, 9, 9, 20, 20],
        tag_widths: vec![8, 8, 12, 12, 12, 12],
        hashing: TageHashing::TageScL,
        banking: Some(TageBanking {
            short_factor: 2,
            long_factor: 3,
            first_long_bank: 2,
            enabled: vec![true, true, true, false, true, true],
        }),
        ..TageConfig::default()
    }
}

#[test]
fn a_bank_pair_shares_its_tag_and_offsets_its_index_by_it() {
    let tage = TageCore::new(&banked_config());

    let (indices, tags) = tage.indices_and_tags(0x8000_1234);

    let slice = |index: usize| index >> 6;
    assert_eq!(tags[1], tags[0]);
    assert_eq!(indices[1] & 63, (indices[0] ^ usize::from(tags[0])) & 63);
    assert_ne!(slice(indices[0]), slice(indices[1]), "each enabled bank its own slice");
}

#[test]
fn enabled_banks_take_consecutive_slices_of_their_array() {
    let tage = TageCore::new(&banked_config());

    let (indices, _) = tage.indices_and_tags(0x8000_1234);

    let slices: Vec<usize> = [2, 4, 5].iter().map(|&bank| indices[bank] >> 6).collect();
    assert_eq!(slices[1], (slices[0] + 1) % 3, "{slices:?}");
    assert_eq!(slices[2], (slices[1] + 1) % 3, "{slices:?}");
    assert!(slices.iter().all(|&slice| slice < 3));
}

#[test]
fn a_disabled_bank_never_provides() {
    let mut tage = TageCore::new(&banked_config());
    let pc = 0x8000_1234;
    let (indices, tags) = tage.indices_and_tags(pc);
    let entry = &mut tage.storage[tage.bank_storage[3]][indices[3]];
    entry.tag = tags[3];
    entry.ctr = 3;

    let prediction = tage.predict(pc);

    assert_ne!(prediction.provider, Some(3));
}

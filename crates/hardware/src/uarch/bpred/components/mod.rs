//! Reusable building blocks and sub-predictors for branch prediction.

pub mod folded_history;
pub mod ittage;
pub mod loop_predictor;
pub mod sc_types;
pub mod stat_corrector;
pub mod tage_core;
pub mod tage_history;
pub mod tagged_bank;

/// Advances a 64-bit xorshift generator and returns its new state.
pub(crate) const fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

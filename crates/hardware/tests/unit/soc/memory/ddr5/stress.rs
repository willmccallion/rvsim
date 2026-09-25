//! Randomized stress: 1024 mixed R/W requests, verify every one gets a
//! response, no debug assertion fires, bandwidth in a wide sanity band.

use crate::unit::soc::memory::ddr5::common::{Harness, addr_from, read_op, tiny_config, write_op};

/// LCG for deterministic pseudo-random test traffic.
fn lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state
}

#[test]
fn stress_1024_requests_all_complete() {
    let cfg = tiny_config();
    let mut h = Harness::new(cfg);
    let mut ids = Vec::with_capacity(1024);
    let mut seed = 0xdead_beef_cafe_babe;
    let mut cycle = 0u64;
    for _ in 0..1024 {
        let r = lcg(&mut seed);
        let bg = ((r >> 3) & 1) as u8;
        let bank = ((r >> 5) & 1) as u8;
        let row = ((r >> 7) & 0xf) as u32;
        let col = ((r >> 12) & 0xf) as u32;
        let is_write = (r & 1) == 1;
        let a = addr_from(&cfg, 0, bg, bank, row, col);
        let op = if is_write { write_op() } else { read_op() };
        ids.push(h.issue(a, cycle, op));
        cycle += 4;
    }
    let mut last_completion = 0u64;
    for id in ids {
        let at = h.response_at(id);
        if at > last_completion {
            last_completion = at;
        }
    }
    // Loose bandwidth sanity: 1024 bursts × BL/2 = 8192 data cycles is
    // the theoretical peak on a single subchannel. Allow up to 32×
    // slack for row misses and refresh overhead.
    let theoretical_min = 1024 * 8;
    let sanity_upper = theoretical_min * 32;
    assert!(
        last_completion >= theoretical_min,
        "completion {last_completion} below theoretical floor {theoretical_min}"
    );
    assert!(
        last_completion <= sanity_upper,
        "completion {last_completion} above sanity ceiling {sanity_upper}"
    );
}

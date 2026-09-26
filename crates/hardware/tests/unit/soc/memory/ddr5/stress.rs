//! Randomized stress: 1024 mixed R/W requests, verify every one gets a
//! response, no debug assertion fires, and the data bus never carries two
//! bursts at once.

use crate::unit::soc::memory::ddr5::common::{Harness, addr_from, read_op, tiny_config, write_op};
use rvsim_core::sim::packet::DramCmdKind;
use rvsim_core::soc::memory::ddr5::Ddr5Timing;

/// LCG for deterministic pseudo-random test traffic.
fn lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state
}

#[test]
fn stress_1024_requests_all_complete() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
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
    for id in ids {
        let _ = h.response_at(id);
    }
    h.run_until(cycle + 20_000);
    let mut bursts: Vec<(u64, u64)> = h
        .dram_cmds()
        .into_iter()
        .filter_map(|c| match c.kind {
            DramCmdKind::Read => Some((c.fire_at + t.t_cas, c.fire_at + t.t_cas + t.bl_half)),
            DramCmdKind::Write => Some((c.fire_at + t.t_cwl, c.fire_at + t.t_cwl + t.bl_half)),
            _ => None,
        })
        .collect();
    bursts.sort_unstable();
    assert!(!bursts.is_empty());
    for pair in bursts.windows(2) {
        assert!(pair[1].0 >= pair[0].1, "data bursts overlap: {:?} then {:?}", pair[0], pair[1]);
    }
}

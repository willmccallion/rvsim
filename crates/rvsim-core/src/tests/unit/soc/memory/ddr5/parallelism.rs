//! Bank / bank-group parallelism: reads to independent banks progress even
//! when the data bus is oversubscribed, and each subsequent burst never
//! overlaps a prior one.

use crate::config::ddr5::Ddr5Timing;
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, controller_latency, read_op, tiny_config,
};

#[test]
fn four_reads_across_bank_groups_serialize_on_data_bus() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Four cold reads, different (bg, bank) combinations, all rank 0.
    let addrs = [
        addr_from(&cfg, 0, 0, 0, 0, 0),
        addr_from(&cfg, 0, 0, 1, 1, 0),
        addr_from(&cfg, 0, 1, 0, 2, 0),
        addr_from(&cfg, 0, 1, 1, 3, 0),
    ];
    // Enqueue all four before letting the scheduler tick, so parallelism
    // across banks is exposed instead of being serialized by the harness.
    let mut ids = Vec::new();
    for a in &addrs {
        ids.push(h.issue(*a, 0, read_op()));
    }
    let resps: Vec<u64> = ids.iter().map(|id| h.response_at(*id)).collect();
    // First burst ends at startup + BL/2.
    let first = resps[0];
    assert_eq!(first, t.t_rcd + t.t_cas + t.bl_half + controller_latency(&cfg));
    // Successive bursts strictly forward — no read completes before an
    // earlier one and each is at least BL/2 later.
    for pair in resps.windows(2) {
        assert!(pair[1] > pair[0], "burst ordering regressed: {:?} then {:?}", pair[0], pair[1]);
        assert!(
            pair[1] - pair[0] >= t.bl_half,
            "burst spacing {} below BL/2 {}",
            pair[1] - pair[0],
            t.bl_half
        );
    }
}

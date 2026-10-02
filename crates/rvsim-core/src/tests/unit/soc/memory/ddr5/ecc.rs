//! Patrol scrubbing: with side-band ECC the controller sweeps DRAM with
//! background reads at the configured rate and never answers them.

use crate::config::ddr5::{Ddr5Timing, EccKind};
use crate::sim::packet::{DramCmdKind, Packet};
use crate::soc::memory::ddr5::{EccPolicy, SideBandEcc};
use crate::tests::unit::soc::memory::ddr5::common::{Harness, tiny_config};

#[test]
fn scrub_interval_resolves_nanoseconds_to_dram_clocks() {
    let t = Ddr5Timing::default();
    let policy = SideBandEcc { patrol_scrub_ns: Some(1_000) };
    assert_eq!(policy.scrub_interval(&t), Some(2400), "1 us is 2400 clocks at 2.4 GHz");
    assert_eq!(SideBandEcc { patrol_scrub_ns: None }.scrub_interval(&t), None);
    assert_eq!(EccKind::None.build().scrub_interval(&t), None);
}

#[test]
fn scrubber_reads_consecutive_lines_and_produces_no_responses() {
    let mut cfg = tiny_config();
    cfg.ecc = EccKind::SecDed { patrol_scrub_ns: Some(100) };
    let mut h = Harness::new(cfg);
    let interval = 240;
    h.run_until(interval * 8 + 200);
    let reads = h.commands_of(DramCmdKind::Read);
    assert!(reads.len() >= 8, "expected at least 8 scrub reads, got {}", reads.len());
    assert_eq!(reads[0].row, 0);
    let responses = {
        let mut count = 0;
        let mut retained = Vec::new();
        while let Some(event) = h.queue.pop_ready(u64::MAX) {
            if matches!(event.packet, Packet::MemResp { .. }) {
                count += 1;
            }
            retained.push(event);
        }
        for evt in retained {
            h.queue.schedule(evt.fire_at, evt.target, evt.source, evt.packet);
        }
        count
    };
    assert_eq!(responses, 0, "scrub reads must not be answered");
}

#[test]
fn no_ecc_means_no_background_traffic() {
    let cfg = tiny_config();
    let mut h = Harness::new(cfg);
    h.run_until(5_000);
    assert!(h.commands_of(DramCmdKind::Read).is_empty());
}

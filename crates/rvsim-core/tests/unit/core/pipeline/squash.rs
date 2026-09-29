//! A partial squash must forget every in-flight load younger than the
//! surviving tag: its response would otherwise write a physical register
//! that a newer instruction may already own.

use rvsim_core::common::{PhysAddr, VirtAddr};
use rvsim_core::sim::components::ReqId;
use rvsim_core::uarch::pipeline::engine::BackendCommon;
use rvsim_core::uarch::pipeline::latches::ExMem1Entry;
use rvsim_core::uarch::pipeline::outstanding::{LoadParts, OutstandingLoad};
use rvsim_core::uarch::pipeline::rob::RobTag;

fn in_flight_load(tag: u32) -> OutstandingLoad {
    OutstandingLoad {
        entry: ExMem1Entry { rob_tag: RobTag(tag), ..ExMem1Entry::default() },
        paddr: PhysAddr::new(0x8000_0000),
        vaddr: VirtAddr::new(0x8000_0000),
        dirty_updates: rvsim_core::arch::translation::DirtyUpdates::NONE,
        side_effecting: false,
        parts: LoadParts::Whole(None),
    }
}

#[test]
fn squash_after_keeps_older_loads_and_drops_younger_ones() {
    let mut common = BackendCommon::default();
    for tag in [10, 11, 12, 13] {
        let _ = common.outstanding_loads.insert(ReqId::new(u64::from(tag)), in_flight_load(tag));
    }
    common.mem1_replay.push(ExMem1Entry { rob_tag: RobTag(14), ..ExMem1Entry::default() });

    common.squash_after(RobTag(11));

    let mut survivors: Vec<u32> =
        common.outstanding_loads.values().map(|l| l.entry.rob_tag.0).collect();
    survivors.sort_unstable();
    assert_eq!(survivors, vec![10, 11], "loads at or before the keep tag survive");
    assert!(common.mem1_replay.is_empty(), "a younger replayed op is dropped too");
}

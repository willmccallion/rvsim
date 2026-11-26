//! The core runs as many page-table walks at once as it has walkers, shared
//! by fetch and data or one set per side, and a squashed walk keeps its
//! walker until its PTE read in flight answers.

use std::num::NonZeroUsize;

use crate::common::{AccessType, Asid, PhysAddr, VirtAddr};
use crate::config::WalkerSharing;
use crate::isa::privileged::PrivilegeMode;
use crate::sim::components::ReqId;
use crate::uarch::mmu::ptw::WalkState;
use crate::uarch::pipeline::engine::{BackendCommon, WalkerPool};
use crate::uarch::pipeline::latches::{ExMem1Entry, InFlightOp};
use crate::uarch::pipeline::outstanding::{
    OutstandingWalk, PageTranslations, QueuedWalk, WalkContinuation, WalkSide,
};
use crate::uarch::pipeline::rob::RobTag;

const PAGE: u64 = 0x4000_0000;

fn backend(sharing: WalkerSharing) -> BackendCommon {
    BackendCommon {
        walkers: WalkerPool { count: NonZeroUsize::MIN, sharing },
        ..BackendCommon::default()
    }
}

/// A data walk of `PAGE` for the load with ROB tag `tag`.
fn data_walk(tag: u32) -> OutstandingWalk {
    OutstandingWalk {
        state: WalkState {
            vaddr: VirtAddr::new(PAGE),
            access: AccessType::Read,
            privilege: PrivilegeMode::Supervisor,
            level: 2,
            ppn_raw: 0,
            asid: Asid::new(0),
        },
        access_size: 8,
        continuation: WalkContinuation::LoadStore {
            entry: ExMem1Entry {
                op: InFlightOp { rob_tag: RobTag::new(tag), ..InFlightOp::default() },
                ..ExMem1Entry::default()
            },
            translations: Box::new(PageTranslations::default()),
        },
    }
}

fn run(common: &mut BackendCommon, walk: OutstandingWalk) {
    let _ = common.memory.outstanding_walks.insert(ReqId::new(1), walk);
}

fn queue(common: &mut BackendCommon, walk: OutstandingWalk) {
    let queued = QueuedWalk { walk, pte_addr: PhysAddr::new(0x8000_0000), earliest: 0 };
    common.memory.walk_queue.push_back(queued);
}

#[test]
fn a_shared_walker_running_a_data_walk_has_no_room_for_a_fetch_walk() {
    let mut common = backend(WalkerSharing::Shared);

    run(&mut common, data_walk(10));

    assert!(!common.has_free_walker(WalkSide::Fetch));
    assert!(!common.has_free_walker(WalkSide::Data));
}

#[test]
fn per_side_walkers_run_a_fetch_walk_beside_a_data_walk() {
    let mut common = backend(WalkerSharing::PerSide);

    run(&mut common, data_walk(10));

    assert!(common.has_free_walker(WalkSide::Fetch));
    assert!(!common.has_free_walker(WalkSide::Data));
}

#[test]
fn a_walk_of_a_page_another_walk_is_reading_waits_for_it() {
    let mut common = backend(WalkerSharing::Shared);

    run(&mut common, data_walk(10));

    assert!(common.memory.walks_page(&data_walk(11)));
}

#[test]
fn a_squashed_walk_keeps_its_walker_and_a_squashed_queued_walk_is_dropped() {
    let mut common = backend(WalkerSharing::Shared);
    run(&mut common, data_walk(14));
    queue(&mut common, data_walk(15));

    common.squash_after(RobTag::new(11));

    let running = &common.memory.outstanding_walks[&ReqId::new(1)];
    assert!(running.is_squashed(), "the walk ends when its PTE read answers");
    assert!(!common.has_free_walker(WalkSide::Data), "until then it holds the walker");
    assert!(common.memory.walk_queue.is_empty(), "a queued walk holds nothing");
    assert!(!common.memory.walks_page_now(&data_walk(16)), "a later miss is not merged into it");
}

#[test]
fn a_squash_leaves_older_walks_alone() {
    let mut common = backend(WalkerSharing::Shared);
    run(&mut common, data_walk(10));
    queue(&mut common, data_walk(11));

    common.squash_after(RobTag::new(11));

    assert!(!common.memory.outstanding_walks[&ReqId::new(1)].is_squashed());
    assert_eq!(common.memory.walk_queue.len(), 1);
}

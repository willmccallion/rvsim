//! Whether fetch or the memory system still has work in flight: each kind
//! of outstanding work alone keeps its side waiting.

use crate::common::{AccessType, Asid, VirtAddr};
use crate::isa::privileged::PrivilegeMode;
use crate::sim::components::ReqId;
use crate::uarch::mmu::ptw::WalkState;
use crate::uarch::pipeline::engine::{BackendCommon, FetchRestart};
use crate::uarch::pipeline::latches::Mem2WbEntry;
use crate::uarch::pipeline::outstanding::{
    OutstandingStore, OutstandingWalk, StoreOwner, WalkContinuation,
};

fn walk() -> OutstandingWalk {
    OutstandingWalk {
        state: WalkState {
            vaddr: VirtAddr::new(0x4000_0000),
            access: AccessType::Read,
            privilege: PrivilegeMode::Supervisor,
            level: 2,
            ppn_raw: 0,
            asid: Asid::new(0),
        },
        access_size: 8,
        continuation: WalkContinuation::Squashed,
    }
}

#[test]
fn an_idle_backend_is_not_waiting() {
    let common = BackendCommon::default();

    assert!(!common.fetch.is_waiting());
    assert!(!common.memory.is_waiting());
}

#[test]
fn a_fetch_parked_on_a_walk_is_waiting() {
    let mut common = BackendCommon::default();

    common.fetch.fetch_restart = FetchRestart::AwaitingWalk;

    assert!(common.fetch.is_waiting());
}

#[test]
fn an_unacknowledged_store_keeps_memory_waiting() {
    let mut common = BackendCommon::default();

    let store = OutstandingStore { owner: StoreOwner::StoreBuffer };
    let _ = common.memory.outstanding_stores.insert(ReqId::new(1), store);

    assert!(common.memory.is_waiting());
}

#[test]
fn a_running_walk_keeps_memory_waiting() {
    let mut common = BackendCommon::default();

    let _ = common.memory.outstanding_walks.insert(ReqId::new(1), walk());

    assert!(common.memory.is_waiting());
}

#[test]
fn a_forwarded_result_awaiting_writeback_keeps_memory_waiting() {
    let mut common = BackendCommon::default();

    common.memory.forwarded_results.push(Mem2WbEntry::default());

    assert!(common.memory.is_waiting());
}

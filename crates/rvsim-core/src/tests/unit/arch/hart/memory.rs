//! # Hart Address Translation Tests
//!
//! Tests for address translation. Cache-walk-latency tests that exercised
//! the deleted `SystemState::simulate_memory_access` synchronous helper were
//! superseded by integration tests that drive the packet-based cache
//! hierarchy through a `Simulator`.

use crate::common::{AccessType, VirtAddr};
use crate::tests::support::harness::minimal_system;
use crate::uarch::mmu::TranslateOutcome;

#[test]
fn a_bare_read_translates_to_its_own_address_without_trapping() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("NeedPte not expected in direct mode")
    };

    assert_eq!(result.trap, None);
    assert_eq!(result.paddr.val(), 0x8000_0000);
}

#[test]
fn bare_reads_on_several_pages_translate_to_the_same_address() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let test_addrs = vec![0x8000_0000u64, 0x8000_1000u64, 0x8000_2000u64];

    for addr in test_addrs {
        let vaddr = VirtAddr::new(addr);
        let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
            panic!("NeedPte not expected in direct mode")
        };
        assert_eq!(result.paddr.val(), addr);
    }
}

#[test]
fn a_bare_fetch_translates_without_a_trap() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Fetch, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert_eq!(result.trap, None);
}

#[test]
fn a_bare_write_translates_without_a_trap() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Write, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert_eq!(result.trap, None);
}

#[test]
fn a_bare_translation_costs_no_cycles() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert_eq!(result.cycles, 0);
}

#[test]
fn repeated_bare_translations_give_the_same_address() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    for _ in 0..5 {
        let vaddr = VirtAddr::new(0x8000_0000);
        let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
            panic!("NeedPte not expected in direct mode")
        };
        assert_eq!(result.paddr.val(), 0x8000_0000);
    }
}

#[test]
fn a_bare_fetch_from_an_unmapped_address_traps() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Fetch, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert!(result.trap.is_some());
}

#[test]
fn a_bare_read_from_an_unmapped_address_traps() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert!(result.trap.is_some());
}

#[test]
fn a_bare_write_to_an_unmapped_address_traps() {
    let mut sys = minimal_system();
    let mut state = sys.core_ctx(0);

    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Write, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert!(result.trap.is_some());
}

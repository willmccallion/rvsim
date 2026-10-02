//! # Hart Address Translation Tests
//!
//! Tests for address translation. Cache-walk-latency tests that exercised
//! the deleted `SystemState::simulate_memory_access` synchronous helper were
//! superseded by integration tests that drive the packet-based cache
//! hierarchy through a `Simulator`.

use crate::common::{AccessType, VirtAddr};
use crate::config::Config;
use crate::system::SystemState;
use crate::uarch::mmu::TranslateOutcome;

fn create_test_cpu() -> SystemState {
    let config = Config::default();
    let mut state = SystemState::build(&config, "");
    state.direct_mode = true;
    state
}

#[test]
fn test_translate_direct_mode_valid_address() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("NeedPte not expected in direct mode")
    };

    assert_eq!(result.trap, None);
    assert_eq!(result.paddr.val(), 0x8000_0000);
}

#[test]
fn test_translate_direct_mode_different_addresses() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

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
fn test_translate_direct_mode_fetch_access() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Fetch, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert_eq!(result.trap, None);
}

#[test]
fn test_translate_direct_mode_write_access() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Write, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert_eq!(result.trap, None);
}

#[test]
fn test_translate_preserves_translation_cost() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert_eq!(result.cycles, 0);
}

#[test]
fn test_translate_multiple_calls() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    for _ in 0..5 {
        let vaddr = VirtAddr::new(0x8000_0000);
        let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
            panic!("NeedPte not expected in direct mode")
        };
        assert_eq!(result.paddr.val(), 0x8000_0000);
    }
}

#[test]
fn test_translate_invalid_address_fetch() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Fetch, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert!(result.trap.is_some());
}

#[test]
fn test_translate_invalid_address_read() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert!(result.trap.is_some());
}

#[test]
fn test_translate_invalid_address_write() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = true;

    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Write, 4) else {
        panic!("NeedPte not expected in direct mode")
    };
    assert!(result.trap.is_some());
}

#[test]
fn test_translate_with_direct_mode_false() {
    let mut sys = create_test_cpu();
    let mut state = sys.core_ctx(0);
    state.direct_mode = false;

    let vaddr = VirtAddr::new(0x8000_0000);
    // In M-mode with no paging (default SATP=Bare), translation completes
    // immediately and identity-maps. Walks only fire under Sv39+.
    let TranslateOutcome::Ready(result) = state.translate(vaddr, AccessType::Read, 4) else {
        panic!("M-mode + Bare should not require a walk")
    };
    assert!(result.trap.is_some() || result.paddr.val() == 0x8000_0000);
}

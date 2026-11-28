//! The host's view of the program's memory: reads, C-string reads and
//! writes that cross page boundaries and stop at their limits.

use crate::common::PhysAddr;
use crate::config::Config;
use crate::isa::csr;
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::system::SystemState;
use crate::tests::support::harness::minimal_system;

const PAGE_SIZE: u64 = 0x1000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const L0_PPN: u64 = 0x8_0102;
const FIRST_VA: u64 = 0x4000_0000;
const SECOND_VA: u64 = FIRST_VA + PAGE_SIZE;
const FIRST_FRAME: u64 = 0x8040_0000;
/// Below the first frame, so the two pages are not physically adjacent.
const SECOND_FRAME: u64 = 0x8030_0000;
const PTE_V: u64 = 1;
const PTE_VRWAD: u64 = 0b1100_0111 | PTE_V;
const FIRST_BYTES: usize = 8;
const SECOND_BYTES: usize = 4;
const STRING_LIMIT: usize = 16;
/// Mid-page, so a write just before it does not end at a page boundary.
const TOHOST: u64 = 0x8000_9100;
const BYTES_BEFORE_TOHOST: u64 = 8;

fn store(sys: &mut SystemState, paddr: u64, bytes: &[u8]) {
    sys.uncore.kernel.memory.load(PhysAddr::new(paddr), bytes);
}

fn store_pte(sys: &mut SystemState, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    store(sys, (table_ppn << 12) | (index * 8), &pte.to_le_bytes());
}

/// A supervisor-mode hart whose page at `FIRST_VA` and the one after map
/// to `FIRST_FRAME` and `SECOND_FRAME`.
fn system_with_split_pages() -> SystemState {
    let mut sys = minimal_system();
    store_pte(&mut sys, ROOT_PPN, FIRST_VA, 2, (L1_PPN << 10) | PTE_V);
    store_pte(&mut sys, L1_PPN, FIRST_VA, 1, (L0_PPN << 10) | PTE_V);
    for (va, frame) in [(FIRST_VA, FIRST_FRAME), (SECOND_VA, SECOND_FRAME)] {
        store_pte(&mut sys, L0_PPN, va, 0, ((frame >> 12) << 10) | PTE_VRWAD);
    }
    let hart = &mut sys.harts[0];
    hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
    hart.privilege = PrivilegeMode::Supervisor;
    sys
}

fn read_physical(sys: &SystemState, paddr: u64, len: usize) -> Vec<u8> {
    sys.uncore.kernel.memory.read_bytes(PhysAddr::new(paddr), len).expect("inside RAM").to_vec()
}

/// `FIRST_BYTES` of `first` then `SECOND_BYTES` of `second`.
fn straddling(first: u8, second: u8) -> Vec<u8> {
    let mut bytes = vec![first; FIRST_BYTES];
    bytes.extend([second; SECOND_BYTES]);
    bytes
}

#[test]
fn a_host_read_across_a_page_boundary_takes_each_part_from_its_own_frame() {
    let mut sys = system_with_split_pages();
    store(&mut sys, FIRST_FRAME + PAGE_SIZE - FIRST_BYTES as u64, &[1; FIRST_BYTES]);
    store(&mut sys, SECOND_FRAME, &[2; SECOND_BYTES]);
    let state = sys.core_ctx(0);

    let bytes = state.read_now(SECOND_VA - FIRST_BYTES as u64, FIRST_BYTES + SECOND_BYTES);

    assert_eq!(bytes, Ok(straddling(1, 2)));
}

#[test]
fn a_host_read_from_the_start_of_a_page_stays_in_that_page() {
    let mut sys = system_with_split_pages();
    store(&mut sys, FIRST_FRAME, &[3; SECOND_BYTES]);
    let state = sys.core_ctx(0);

    assert_eq!(state.read_now(FIRST_VA, SECOND_BYTES), Ok(vec![3; SECOND_BYTES]));
}

#[test]
fn a_host_write_across_a_page_boundary_lands_each_part_in_its_own_frame() {
    let mut sys = system_with_split_pages();
    let mut state = sys.core_ctx(0);

    let written = state.write_now(SECOND_VA - FIRST_BYTES as u64, &straddling(1, 2));

    assert_eq!(written, Ok(()));
    let first_part = read_physical(&sys, FIRST_FRAME + PAGE_SIZE - FIRST_BYTES as u64, FIRST_BYTES);
    assert_eq!(first_part, vec![1; FIRST_BYTES]);
    assert_eq!(read_physical(&sys, SECOND_FRAME, SECOND_BYTES), vec![2; SECOND_BYTES]);
}

/// A machine-mode hart, which reaches memory untranslated, with HTIF's
/// window at `TOHOST` inside RAM.
fn system_with_htif() -> SystemState {
    let mut config = Config::minimal();
    config.system.tohost_addr = Some(PhysAddr::new(TOHOST));
    SystemState::build(&config)
}

#[test]
fn a_host_write_into_the_htif_window_writes_the_bytes_before_it_and_faults_at_it() {
    let mut sys = system_with_htif();
    let start = TOHOST - BYTES_BEFORE_TOHOST;

    let written = sys.core_ctx(0).write_now(start, &[0x5a; 2 * BYTES_BEFORE_TOHOST as usize]);

    assert_eq!(written, Err(Trap::StoreAccessFault(TOHOST)));
    assert_eq!(read_physical(&sys, start, BYTES_BEFORE_TOHOST as usize), [0x5a; 8]);
}

#[test]
fn a_string_longer_than_its_limit_is_cut_at_the_limit() {
    let mut sys = minimal_system();
    let start = sys.uncore.kernel.config.system.ram_base.val();
    store(&mut sys, start, &[b'a'; 2 * STRING_LIMIT]);
    sys.harts[0].privilege = PrivilegeMode::Machine;
    let state = sys.core_ctx(0);

    let string = state.read_cstr_now(start, STRING_LIMIT);

    assert_eq!(string, Ok(vec![b'a'; STRING_LIMIT]));
}

#[test]
fn a_string_that_crosses_a_page_and_exceeds_its_limit_is_cut_at_the_limit() {
    let mut sys = system_with_split_pages();
    store(&mut sys, FIRST_FRAME + PAGE_SIZE - FIRST_BYTES as u64, &[b'a'; FIRST_BYTES]);
    store(&mut sys, SECOND_FRAME, &[b'b'; 2 * STRING_LIMIT]);
    let state = sys.core_ctx(0);

    let string = state.read_cstr_now(SECOND_VA - FIRST_BYTES as u64, STRING_LIMIT);

    let mut expected = vec![b'a'; FIRST_BYTES];
    expected.extend([b'b'; STRING_LIMIT - FIRST_BYTES]);
    assert_eq!(string, Ok(expected));
}

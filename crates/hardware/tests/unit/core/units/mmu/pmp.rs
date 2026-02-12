//! PMP (Physical Memory Protection) Unit Tests.
//!
//! Verifies address matching (TOR, NA4, NAPOT), permission checks,
//! M-mode bypass logic, and locking behaviour per RISC-V spec §3.7.

use riscv_core::core::units::mmu::pmp::{Pmp, PmpAddrMatch, PmpEntry, PmpResult};

// ══════════════════════════════════════════════════════════
// Constants
// ══════════════════════════════════════════════════════════

// Configuration byte helpers
const R: u8 = 1 << 0;
const W: u8 = 1 << 1;
const X: u8 = 1 << 2;
const A_TOR: u8 = 1 << 3; // A = TOR (01 << 3)
const A_NA4: u8 = 2 << 3; // A = NA4 (10 << 3)
const A_NAPOT: u8 = 3 << 3; // A = NAPOT (11 << 3)
const L: u8 = 1 << 7; // Lock bit

// ══════════════════════════════════════════════════════════
// 1. Address Match Mode Decoding
// ══════════════════════════════════════════════════════════

#[test]
fn addr_match_off() {
    assert_eq!(PmpAddrMatch::from_bits(0), PmpAddrMatch::Off);
}

#[test]
fn addr_match_tor() {
    assert_eq!(PmpAddrMatch::from_bits(1), PmpAddrMatch::Tor);
}

#[test]
fn addr_match_na4() {
    assert_eq!(PmpAddrMatch::from_bits(2), PmpAddrMatch::Na4);
}

#[test]
fn addr_match_napot() {
    assert_eq!(PmpAddrMatch::from_bits(3), PmpAddrMatch::Napot);
}

// ══════════════════════════════════════════════════════════
// 2. M-mode bypass when no entries configured
// ══════════════════════════════════════════════════════════

#[test]
fn machine_mode_full_access_no_entries() {
    let pmp = Pmp::new();
    // M-mode should have full access when no PMP entries are active.
    let result = pmp.check(0x8000_0000, 4, true, false, false, true);
    assert_eq!(result, PmpResult::Allow);
}

#[test]
fn machine_mode_write_no_entries() {
    let pmp = Pmp::new();
    let result = pmp.check(0x1000, 8, false, true, false, true);
    assert_eq!(result, PmpResult::Allow);
}

#[test]
fn machine_mode_exec_no_entries() {
    let pmp = Pmp::new();
    let result = pmp.check(0x0, 4, false, false, true, true);
    assert_eq!(result, PmpResult::Allow);
}

// ══════════════════════════════════════════════════════════
// 3. Non-M-mode gets NoMatch when no entries configured
// ══════════════════════════════════════════════════════════

#[test]
fn user_mode_no_match_no_entries() {
    let pmp = Pmp::new();
    let result = pmp.check(0x8000_0000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);
}

// ══════════════════════════════════════════════════════════
// 4. TOR region
// ══════════════════════════════════════════════════════════

#[test]
fn tor_permits_access_in_range() {
    let mut pmp = Pmp::new();
    // Entry 0: TOR with addr = 0x2000 → range [0, 0x8000)
    // pmpaddr is byte_addr >> 2
    pmp.set_addr(0, 0x2000); // byte addr 0x8000
    pmp.set_cfg(0, A_TOR | R | W | X);

    // Access at byte addr 0x4000, size 4 → within [0, 0x8000)
    let result = pmp.check(0x4000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);
}

#[test]
fn tor_denies_access_outside_range() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000); // range [0, 0x8000)
    pmp.set_cfg(0, A_TOR | R | W | X);

    // Access at byte addr 0x8000 → at the boundary, not inside [0, 0x8000)
    let result = pmp.check(0x8000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);
}

#[test]
fn tor_range_between_entries() {
    let mut pmp = Pmp::new();
    // Entry 0: addr = 0x1000 (byte 0x4000) — just sets the low bound for entry 1
    pmp.set_addr(0, 0x1000);
    pmp.set_cfg(0, 0); // disabled (A = Off)

    // Entry 1: TOR, addr = 0x2000 → range [0x4000, 0x8000)
    pmp.set_addr(1, 0x2000);
    pmp.set_cfg(1, A_TOR | R);

    // Inside range
    let result = pmp.check(0x5000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    // Below range
    let result = pmp.check(0x3000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);
}

#[test]
fn tor_denies_write_when_only_read_permitted() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000); // [0, 0x8000)
    pmp.set_cfg(0, A_TOR | R); // read-only

    let read_result = pmp.check(0x1000, 4, true, false, false, false);
    assert_eq!(read_result, PmpResult::Allow);

    let write_result = pmp.check(0x1000, 4, false, true, false, false);
    assert_eq!(write_result, PmpResult::Deny);
}

// ══════════════════════════════════════════════════════════
// 5. NA4 region
// ══════════════════════════════════════════════════════════

#[test]
fn na4_matches_exactly_four_bytes() {
    let mut pmp = Pmp::new();
    // pmpaddr = 0x1000 → byte base = 0x4000, size = 4 → [0x4000, 0x4004)
    pmp.set_addr(0, 0x1000);
    pmp.set_cfg(0, A_NA4 | R | W);

    // Exact match
    let result = pmp.check(0x4000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    // One byte inside
    let result = pmp.check(0x4000, 1, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    // Just outside
    let result = pmp.check(0x4004, 1, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);

    // Overlapping the end
    let result = pmp.check(0x4002, 4, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);
}

// ══════════════════════════════════════════════════════════
// 6. NAPOT region
// ══════════════════════════════════════════════════════════

#[test]
fn napot_8_byte_region() {
    let mut pmp = Pmp::new();
    // 8-byte NAPOT: trailing ones = 0 → size = 2^(0+3) = 8
    // pmpaddr = base_addr >> 2, with 0 trailing ones
    // For base = 0x1000: pmpaddr = 0x400
    // But NAPOT encoding: pmpaddr = (base >> 2) | 0 → trailing = 0 means no trailing 1s
    // Actually for 8 bytes: size = 8, trailing ones in pmpaddr = 0
    // pmpaddr = (base >> 2) | ((size/8 - 1)) where size=8 → pmpaddr = base>>2 | 0
    // Wait, let me re-derive:
    //   size = 2^(trailing_ones + 3)
    //   For 8 bytes: trailing_ones = 0, so pmpaddr has 0 trailing 1s
    //   base = pmpaddr << 2 (with mask)
    // pmpaddr for [0x1000, 0x1008): 0x1000 >> 2 = 0x400, and no trailing 1s
    pmp.set_addr(0, 0x400);
    pmp.set_cfg(0, A_NAPOT | R | W | X);

    // Inside [0x1000, 0x1008)
    let result = pmp.check(0x1000, 4, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    let result = pmp.check(0x1004, 4, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    // Outside
    let result = pmp.check(0x1008, 1, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);
}

#[test]
fn napot_larger_region() {
    let mut pmp = Pmp::new();
    // For a 32-byte region [0x1000, 0x1020):
    //   size = 32 = 2^5 → trailing_ones = 5-3 = 2
    //   pmpaddr = (0x1000 >> 2) | 0b11 = 0x400 | 0x3 = 0x403
    pmp.set_addr(0, 0x403);
    pmp.set_cfg(0, A_NAPOT | R);

    let result = pmp.check(0x1000, 1, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    let result = pmp.check(0x101F, 1, true, false, false, false);
    assert_eq!(result, PmpResult::Allow);

    let result = pmp.check(0x1020, 1, true, false, false, false);
    assert_eq!(result, PmpResult::NoMatch);
}

// ══════════════════════════════════════════════════════════
// 7. Permission checks
// ══════════════════════════════════════════════════════════

#[test]
fn read_only_denies_write_and_exec() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000);
    pmp.set_cfg(0, A_TOR | R);

    assert_eq!(
        pmp.check(0x1000, 4, true, false, false, false),
        PmpResult::Allow
    );
    assert_eq!(
        pmp.check(0x1000, 4, false, true, false, false),
        PmpResult::Deny
    );
    assert_eq!(
        pmp.check(0x1000, 4, false, false, true, false),
        PmpResult::Deny
    );
}

#[test]
fn execute_only_denies_read_and_write() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000);
    pmp.set_cfg(0, A_TOR | X);

    assert_eq!(
        pmp.check(0x1000, 4, false, false, true, false),
        PmpResult::Allow
    );
    assert_eq!(
        pmp.check(0x1000, 4, true, false, false, false),
        PmpResult::Deny
    );
    assert_eq!(
        pmp.check(0x1000, 4, false, true, false, false),
        PmpResult::Deny
    );
}

#[test]
fn rwx_permits_all() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000);
    pmp.set_cfg(0, A_TOR | R | W | X);

    assert_eq!(
        pmp.check(0x1000, 4, true, false, false, false),
        PmpResult::Allow
    );
    assert_eq!(
        pmp.check(0x1000, 4, false, true, false, false),
        PmpResult::Allow
    );
    assert_eq!(
        pmp.check(0x1000, 4, false, false, true, false),
        PmpResult::Allow
    );
}

// ══════════════════════════════════════════════════════════
// 8. Locked entries
// ══════════════════════════════════════════════════════════

#[test]
fn locked_entry_applies_to_machine_mode() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000); // [0, 0x8000)
    pmp.set_cfg(0, A_TOR | R | L); // read-only + locked

    // M-mode write should be denied because entry is locked
    let result = pmp.check(0x1000, 4, false, true, false, true);
    assert_eq!(result, PmpResult::Deny);

    // M-mode read should still work
    let result = pmp.check(0x1000, 4, true, false, false, true);
    assert_eq!(result, PmpResult::Allow);
}

#[test]
fn unlocked_entry_m_mode_bypasses() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000); // [0, 0x8000)
    pmp.set_cfg(0, A_TOR | R); // read-only, NOT locked

    // M-mode write should be allowed because entry is not locked
    let result = pmp.check(0x1000, 4, false, true, false, true);
    assert_eq!(result, PmpResult::Allow);
}

#[test]
fn locked_entry_cannot_be_modified() {
    let mut pmp = Pmp::new();
    pmp.set_addr(0, 0x2000);
    pmp.set_cfg(0, A_TOR | R | L); // locked

    // Attempt to change cfg — should be ignored
    pmp.set_cfg(0, A_TOR | R | W | X);
    assert_eq!(pmp.get_cfg(0), A_TOR | R | L);

    // Attempt to change addr — should be ignored
    pmp.set_addr(0, 0x4000);
    assert_eq!(pmp.get_addr(0), 0x2000);
}

// ══════════════════════════════════════════════════════════
// 9. Priority: first matching entry wins
// ══════════════════════════════════════════════════════════

#[test]
fn first_matching_entry_wins() {
    let mut pmp = Pmp::new();
    // Entry 0: [0, 0x8000) — read only
    pmp.set_addr(0, 0x2000);
    pmp.set_cfg(0, A_TOR | R);

    // Entry 1: [0, 0x10000) — read/write
    pmp.set_addr(1, 0x4000);
    pmp.set_cfg(1, A_TOR | R | W);

    // Address 0x1000 matches entry 0 first — write should be denied
    let result = pmp.check(0x1000, 4, false, true, false, false);
    assert_eq!(result, PmpResult::Deny);
}

// ══════════════════════════════════════════════════════════
// 10. PmpEntry field accessors
// ══════════════════════════════════════════════════════════

#[test]
fn pmp_entry_permission_accessors() {
    let entry = PmpEntry {
        cfg: R | W | X | L,
        addr: 0,
    };
    assert!(entry.is_readable());
    assert!(entry.is_writable());
    assert!(entry.is_executable());
    assert!(entry.is_locked());

    let entry2 = PmpEntry { cfg: R, addr: 0 };
    assert!(entry2.is_readable());
    assert!(!entry2.is_writable());
    assert!(!entry2.is_executable());
    assert!(!entry2.is_locked());
}

#[test]
fn pmp_entry_match_mode_accessor() {
    let entry_off = PmpEntry { cfg: 0, addr: 0 };
    assert_eq!(entry_off.match_mode(), PmpAddrMatch::Off);

    let entry_tor = PmpEntry {
        cfg: A_TOR,
        addr: 0,
    };
    assert_eq!(entry_tor.match_mode(), PmpAddrMatch::Tor);

    let entry_na4 = PmpEntry {
        cfg: A_NA4,
        addr: 0,
    };
    assert_eq!(entry_na4.match_mode(), PmpAddrMatch::Na4);

    let entry_napot = PmpEntry {
        cfg: A_NAPOT,
        addr: 0,
    };
    assert_eq!(entry_napot.match_mode(), PmpAddrMatch::Napot);
}

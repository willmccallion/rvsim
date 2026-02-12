//! Unit tests for system-wide constants.
//!
//! This module verifies that global constants are defined with correct values
//! and maintain expected mathematical relationships.

use riscv_core::common::constants::*;

#[test]
fn test_page_size_is_4kb() {
    assert_eq!(PAGE_SIZE, 4096, "PAGE_SIZE should be 4KB");
}

#[test]
fn test_page_shift_matches_page_size() {
    assert_eq!(
        1u64 << PAGE_SHIFT,
        PAGE_SIZE,
        "PAGE_SHIFT should produce PAGE_SIZE when shifted"
    );
}

#[test]
fn test_page_offset_mask_correct_value() {
    // This test catches mutants that replace - with + or /
    assert_eq!(
        PAGE_OFFSET_MASK, 0xFFF,
        "PAGE_OFFSET_MASK should be 0xFFF (4095)"
    );
    assert_eq!(
        PAGE_OFFSET_MASK,
        PAGE_SIZE - 1,
        "PAGE_OFFSET_MASK should equal PAGE_SIZE - 1"
    );
}

#[test]
fn test_page_offset_mask_extracts_offset() {
    // Verify that the mask correctly extracts only the offset bits
    let address: u64 = 0x1234567;
    let offset = address & PAGE_OFFSET_MASK;

    // The offset should be less than PAGE_SIZE
    assert!(offset < PAGE_SIZE, "Offset should be less than PAGE_SIZE");

    // The offset should preserve the lower 12 bits
    assert_eq!(
        offset,
        address & 0xFFF,
        "Mask should preserve lower 12 bits"
    );
}

#[test]
fn test_page_offset_mask_clears_upper_bits() {
    // Test that masking an address removes page number information
    let address = 0x5000 | 0x123; // Page 5, offset 0x123
    let offset = address & PAGE_OFFSET_MASK;
    assert_eq!(offset, 0x123, "Mask should extract only the offset");
}

#[test]
fn test_vpn_mask_value() {
    assert_eq!(
        VPN_MASK, 0x7FFFFFF,
        "VPN_MASK should be 0x7FFFFFF (27 bits)"
    );
}

#[test]
fn test_instruction_masks_and_shifts() {
    assert_eq!(OPCODE_MASK, 0x7F, "Opcode mask should be 7 bits");
    assert_eq!(RD_MASK, 0x1F, "RD mask should be 5 bits");
    assert_eq!(RD_SHIFT, 7, "RD field starts at bit 7");
    assert_eq!(RS1_MASK, 0x1F, "RS1 mask should be 5 bits");
    assert_eq!(RS1_SHIFT, 15, "RS1 field starts at bit 15");
}

#[test]
fn test_compressed_instruction_constants() {
    assert_eq!(INSTRUCTION_SIZE_16, 2, "Compressed instruction is 2 bytes");
    assert_eq!(INSTRUCTION_SIZE_32, 4, "Standard instruction is 4 bytes");
    assert_eq!(
        COMPRESSED_INSTRUCTION_MASK, 0x3,
        "Compressed check uses lower 2 bits"
    );
    assert_eq!(
        COMPRESSED_INSTRUCTION_VALUE, 0x3,
        "Compressed instruction has both lower bits set"
    );
}

#[test]
fn test_delegation_bit_positions() {
    // Verify delegation bit positions are distinct
    let bits = [
        DELEG_SSIP_BIT,
        DELEG_MSIP_BIT,
        DELEG_STIP_BIT,
        DELEG_MTIP_BIT,
        DELEG_SEIP_BIT,
        DELEG_MEIP_BIT,
    ];

    for (i, &bit1) in bits.iter().enumerate() {
        for (j, &bit2) in bits.iter().enumerate() {
            if i != j {
                assert_ne!(bit1, bit2, "Delegation bits should be unique");
            }
        }
    }

    // Verify specific values from RISC-V spec
    assert_eq!(DELEG_SSIP_BIT, 1, "SSIP is bit 1");
    assert_eq!(DELEG_MSIP_BIT, 3, "MSIP is bit 3");
    assert_eq!(DELEG_STIP_BIT, 5, "STIP is bit 5");
    assert_eq!(DELEG_MTIP_BIT, 7, "MTIP is bit 7");
    assert_eq!(DELEG_SEIP_BIT, 9, "SEIP is bit 9");
    assert_eq!(DELEG_MEIP_BIT, 11, "MEIP is bit 11");
}

#[test]
fn test_cause_interrupt_bit() {
    assert_eq!(
        CAUSE_INTERRUPT_BIT,
        1u64 << 63,
        "Interrupt bit should be MSB (bit 63)"
    );

    // Verify that setting this bit indicates an interrupt
    let exception_code = 5u64;
    let interrupt_code = CAUSE_INTERRUPT_BIT | exception_code;

    assert_eq!(
        interrupt_code & CAUSE_INTERRUPT_BIT,
        CAUSE_INTERRUPT_BIT,
        "Interrupt bit should be set"
    );
    assert_eq!(
        exception_code & CAUSE_INTERRUPT_BIT,
        0,
        "Exception should not have interrupt bit"
    );
}

#[test]
fn test_wfi_instruction_value() {
    assert_eq!(
        WFI_INSTRUCTION, 0x10500073,
        "WFI instruction has specific encoding"
    );
}

#[test]
fn test_simulation_constants() {
    assert_eq!(
        HANG_DETECTION_THRESHOLD, 5000,
        "Hang detection triggers after 5000 cycles"
    );
    assert_eq!(
        STATUS_UPDATE_INTERVAL, 5_000_000,
        "Status updates every 5 million cycles"
    );
}

#[test]
fn test_debug_pc_disabled() {
    assert_eq!(DEBUG_PC_START, 0, "Debug PC start is disabled");
    assert_eq!(DEBUG_PC_END, 0, "Debug PC end is disabled");
}

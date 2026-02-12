//! Global System Constants.
//!
//! This module defines system-wide constants used across the simulator. It includes:
//! 1. **Memory Constants:** Page sizes, masks, and shifts for memory management.
//! 2. **Instruction Constants:** Opcode masks and field shifts for instruction decoding.
//! 3. **Delegation Constants:** Bit positions for interrupt delegation.
//! 4. **Simulation Constants:** Thresholds and intervals for simulation control.

/// Page size in bytes (4KB).
pub const PAGE_SIZE: u64 = 4096;

/// Number of bits to shift to convert between bytes and pages.
pub const PAGE_SHIFT: u64 = 12;

/// Mask for extracting the virtual page number (VPN) from an address.
pub const VPN_MASK: u64 = 0x7FFFFFF;

/// Mask for extracting the page offset from an address.
pub const PAGE_OFFSET_MASK: u64 = PAGE_SIZE - 1;

/// Bit mask for extracting the opcode field from a RISC-V instruction.
pub const OPCODE_MASK: u32 = 0x7F;

/// Size of a compressed (16-bit) RISC-V instruction in bytes.
pub const INSTRUCTION_SIZE_16: u64 = 2;

/// Size of a standard (32-bit) RISC-V instruction in bytes.
pub const INSTRUCTION_SIZE_32: u64 = 4;

/// Bit mask for checking if an instruction is compressed.
pub const COMPRESSED_INSTRUCTION_MASK: u16 = 0x3;

/// Value indicating a compressed instruction when masked.
pub const COMPRESSED_INSTRUCTION_VALUE: u16 = 0x3;

/// Bit mask for extracting the destination register (rd) field.
pub const RD_MASK: u32 = 0x1F;

/// Bit position shift for the destination register (rd) field.
pub const RD_SHIFT: u32 = 7;

/// Bit mask for extracting the first source register (rs1) field.
pub const RS1_MASK: u32 = 0x1F;

/// Bit position shift for the first source register (rs1) field.
pub const RS1_SHIFT: u32 = 15;

/// Bit position for machine external interrupt delegation in `mideleg`.
pub const DELEG_MEIP_BIT: u64 = 11;

/// Bit position for machine software interrupt delegation in `mideleg`.
pub const DELEG_MSIP_BIT: u64 = 3;

/// Bit position for machine timer interrupt delegation in `mideleg`.
pub const DELEG_MTIP_BIT: u64 = 7;

/// Bit position for supervisor external interrupt delegation in `mideleg`.
pub const DELEG_SEIP_BIT: u64 = 9;

/// Bit position for supervisor software interrupt delegation in `mideleg`.
pub const DELEG_SSIP_BIT: u64 = 1;

/// Bit position for supervisor timer interrupt delegation in `mideleg`.
pub const DELEG_STIP_BIT: u64 = 5;

/// Bit mask indicating that a trap cause represents an interrupt.
pub const CAUSE_INTERRUPT_BIT: u64 = 1 << 63;

/// Starting program counter address for debug mode (disabled when 0).
pub const DEBUG_PC_START: u64 = 0;

/// Ending program counter address for debug mode (disabled when 0).
pub const DEBUG_PC_END: u64 = 0;

/// Maximum number of cycles before hang detection triggers.
pub const HANG_DETECTION_THRESHOLD: u64 = 5000;

/// Opcode for the Wait For Interrupt (WFI) instruction.
pub const WFI_INSTRUCTION: u32 = 0x10500073;

/// Number of cycles between status update messages during simulation.
pub const STATUS_UPDATE_INTERVAL: u64 = 5_000_000;

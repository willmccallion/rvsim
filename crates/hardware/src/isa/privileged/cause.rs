//! RISC-V Trap Cause Codes.
//!
//! This module defines the cause codes used in the `mcause` and `scause` Control
//! and Status Registers (CSRs). The most significant bit (MSB) indicates whether
//! the trap is an interrupt (1) or an exception (0).
//!
//! The constants here represent the exception code (lower bits) combined with
//! the interrupt bit where applicable.

/// Interrupt definitions (MSB = 1).
pub mod interrupt {
    /// User software interrupt.
    pub const USER_SOFTWARE: u64 = 0x8000_0000_0000_0000;

    /// Supervisor software interrupt.
    pub const SUPERVISOR_SOFTWARE: u64 = 0x8000_0000_0000_0001;

    /// Machine software interrupt.
    pub const MACHINE_SOFTWARE: u64 = 0x8000_0000_0000_0003;

    /// User timer interrupt.
    pub const USER_TIMER: u64 = 0x8000_0000_0000_0004;

    /// Supervisor timer interrupt.
    pub const SUPERVISOR_TIMER: u64 = 0x8000_0000_0000_0005;

    /// Machine timer interrupt.
    pub const MACHINE_TIMER: u64 = 0x8000_0000_0000_0007;

    /// User external interrupt.
    pub const USER_EXTERNAL: u64 = 0x8000_0000_0000_0008;

    /// Supervisor external interrupt.
    pub const SUPERVISOR_EXTERNAL: u64 = 0x8000_0000_0000_0009;

    /// Machine external interrupt.
    pub const MACHINE_EXTERNAL: u64 = 0x8000_0000_0000_000B;
}

/// Exception definitions (MSB = 0).
pub mod exception {
    /// Instruction address misaligned (0).
    pub const INSTRUCTION_ADDRESS_MISALIGNED: u64 = 0;
    /// Instruction access fault (1).
    pub const INSTRUCTION_ACCESS_FAULT: u64 = 1;
    /// Illegal instruction (2).
    pub const ILLEGAL_INSTRUCTION: u64 = 2;
    /// Breakpoint (3).
    pub const BREAKPOINT: u64 = 3;
    /// Load address misaligned (4).
    pub const LOAD_ADDRESS_MISALIGNED: u64 = 4;
    /// Load access fault (5).
    pub const LOAD_ACCESS_FAULT: u64 = 5;
    /// Store/AMO address misaligned (6).
    pub const STORE_ADDRESS_MISALIGNED: u64 = 6;
    /// Store/AMO access fault (7).
    pub const STORE_ACCESS_FAULT: u64 = 7;
    /// Environment call from U-mode (8).
    pub const ENVIRONMENT_CALL_FROM_U_MODE: u64 = 8;
    /// Environment call from S-mode (9).
    pub const ENVIRONMENT_CALL_FROM_S_MODE: u64 = 9;
    /// Environment call from M-mode (11).
    pub const ENVIRONMENT_CALL_FROM_M_MODE: u64 = 11;
    /// Instruction page fault (12).
    pub const INSTRUCTION_PAGE_FAULT: u64 = 12;
    /// Load page fault (13).
    pub const LOAD_PAGE_FAULT: u64 = 13;
    /// Store/AMO page fault (15).
    pub const STORE_PAGE_FAULT: u64 = 15;
    /// Hardware error (18) - Reserved in standard, often used for bus errors.
    pub const HARDWARE_ERROR: u64 = 18;
}

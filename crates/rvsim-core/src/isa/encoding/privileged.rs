//! RISC-V Privileged Architecture Opcodes.
//!
//! Defines opcodes and function codes for system instructions, including
//! CSR access, environment calls, and trap returns.

/// System instruction opcode (0b1110011).
/// Used for CSR instructions, ECALL, EBREAK, xRET, WFI, etc.
pub const OP_SYSTEM: u32 = 0b1110011;

/// Environment Call (ECALL).
/// Traps to a higher privilege level.
pub const ECALL: u32 = 0x0000_0073;

/// Environment Break (EBREAK).
/// Used by debuggers to cause a breakpoint trap.
pub const EBREAK: u32 = 0x0010_0073;

/// Machine Return (MRET).
/// Returns from M-mode trap handler.
pub const MRET: u32 = 0x3020_0073;

/// Supervisor Return (SRET).
/// Returns from S-mode trap handler.
pub const SRET: u32 = 0x1020_0073;

/// Wait for Interrupt (WFI).
/// Stalls the processor until an interrupt occurs.
pub const WFI: u32 = 0x1050_0073;

/// Supervisor Memory-Management Fence (SFENCE.VMA).
/// Flushes TLB entries.
pub const SFENCE_VMA: u32 = 0x1200_0073;

/// Atomic Read/Write CSR (CSRRW).
pub const CSRRW: u32 = 0b001;
/// Atomic Read and Set Bits in CSR (CSRRS).
pub const CSRRS: u32 = 0b010;
/// Atomic Read and Clear Bits in CSR (CSRRC).
pub const CSRRC: u32 = 0b011;
/// Atomic Read/Write CSR Immediate (CSRRWI).
pub const CSRRWI: u32 = 0b101;
/// Atomic Read and Set Bits in CSR Immediate (CSRRSI).
pub const CSRRSI: u32 = 0b110;
/// Atomic Read and Clear Bits in CSR Immediate (CSRRCI).
pub const CSRRCI: u32 = 0b111;

/// System Exit code (used in testing/direct mode).
pub const SYS_EXIT: u64 = 93;

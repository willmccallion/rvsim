//! RISC-V Zicboz (cache-block zero) extension constants.
//!
//! Zicboz adds a single instruction, `cbo.zero rs1`, encoded in the MISC-MEM
//! group with funct3 = CBO. The 12-bit immediate selects the specific CBO
//! operation; `cbo.zero` uses 0x004.
//!
//! Zicbom (cache-block management: `cbo.inval` / `cbo.clean` / `cbo.flush`)
//! shares the funct3=CBO encoding. Each travels as a maintenance operation
//! through every cache level and every other hart's caches to memory.

/// `imm[11:0]` value that selects `cbo.zero` within the MISC-MEM CBO group.
pub const CBO_ZERO_IMM: i64 = 0x004;

/// `imm[11:0]` for `cbo.inval` (Zicbom).
pub const CBO_INVAL_IMM: i64 = 0x000;

/// `imm[11:0]` for `cbo.clean` (Zicbom).
pub const CBO_CLEAN_IMM: i64 = 0x001;

/// `imm[11:0]` for `cbo.flush` (Zicbom).
pub const CBO_FLUSH_IMM: i64 = 0x002;

/// Cache-block size in bytes every cache-block operation acts on.
///
/// Implementation-defined per the spec; 64, and every cache line must be at
/// least this large.
/// Software discovers the value via the `riscv,cboz-block-size` device-tree
/// property — no CSR exposes it directly.
pub const CBOZ_BLOCK_SIZE: u64 = 64;

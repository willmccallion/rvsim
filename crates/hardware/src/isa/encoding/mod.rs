//! Opcode and function-field constants, one module per extension.

/// Base integer instruction set (64-bit RISC-V core instructions).
pub mod rv64i;

/// Integer multiply/divide extension (MUL, DIV, REM instructions).
pub mod rv64m;

/// Atomic memory operations extension (AMO instructions).
pub mod rv64a;

/// Single-precision floating-point extension (32-bit FP operations).
pub mod rv64f;

/// Double-precision floating-point extension (64-bit FP operations).
pub mod rv64d;

/// Half-precision floating-point extension (Zfh, 16-bit FP operations).
pub mod rv64zfh;

/// Bit-manipulation and scalar cryptography extensions (Zba, Zbb, Zbc, Zbs, Zbkb, Zbkx).
pub mod rv64bk;

/// Compressed instruction quadrant and opcode constants.
pub mod rvc;

/// Vector extension (RVV 1.0).
pub mod rvv;

/// System instruction opcodes (ECALL, EBREAK, xRET, FENCE).
pub mod privileged;

/// Cache-block management extensions (Zicbom, Zicboz).
pub mod zicboz;

//! What the RISC-V ISA defines.
//!
//! Encodings, instruction fields, and the vocabulary the rest of the
//! simulator speaks. Nothing here holds state.

/// ISA capability configuration (`IsaConfig`, `VectorIsa`, future families).
pub mod config;

/// CSR addresses.
pub mod csr;

/// Instruction disassembler for debug tracing and diagnostics.
pub mod disasm;

/// Opcode and function-field constants, one module per extension.
pub mod encoding;

/// Fence ordering sets.
pub mod fence;

/// Floating-point rounding modes and exception flags.
pub mod fp;

/// Instruction sizes, field extraction, and decoding into fields.
pub mod instruction;

/// The operations instructions perform, as the decoder names them.
pub mod op;

/// Privileged architecture: privilege modes, traps, and cause codes.
pub mod privileged;

/// Architectural register indices and ABI names.
pub mod reg;

/// Expansion of 16-bit compressed instructions into their 32-bit forms.
pub mod rvc;

/// Vector extension (RVV 1.0) vocabulary: element widths, register groups, vtype.
pub mod rvv;

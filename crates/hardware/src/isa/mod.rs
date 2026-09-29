//! What the RISC-V ISA defines.
//!
//! Encodings, instruction fields, and the vocabulary the rest of the
//! simulator speaks. Nothing here holds state.

/// Application Binary Interface (ABI) register name mappings.
pub mod abi;

/// ISA capability configuration (`IsaConfig`, `VectorIsa`, future families).
pub mod config;

/// Instruction decoding logic for all RISC-V instruction formats.
pub mod decode;

/// Instruction disassembler for debug tracing and diagnostics.
pub mod disasm;

/// Vector instruction disassembler (RVV 1.0).
pub mod disasm_vec;

/// Opcode and function-field constants, one module per extension.
pub mod encoding;

/// Instruction encoding structures and bit extraction utilities.
pub mod instruction;

/// Privileged architecture definitions (trap causes).
pub mod privileged;

/// Expansion of 16-bit compressed instructions into their 32-bit forms.
pub mod rvc;

/// Vector vocabulary: element widths, register groups, vtype.
pub mod vector;

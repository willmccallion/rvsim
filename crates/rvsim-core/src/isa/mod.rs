//! What the RISC-V ISA defines.
//!
//! Encodings, instruction fields, and the vocabulary the rest of the
//! simulator speaks. Nothing here holds state.

#![expect(
    clippy::cast_possible_truncation,
    reason = "instruction fields, immediates and CSR values are bit slices of fixed-width words"
)]

pub mod bits;

pub mod config;

pub mod csr;

pub mod encode;

pub mod encoding;

pub mod extension;

pub mod fence;

pub mod fp;

pub mod instruction;

pub mod misa;

pub mod op;

pub mod privileged;

pub mod reg;

pub mod rvc;

pub mod rvv;

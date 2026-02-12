//! Execution units and functional components.
//!
//! This module contains implementations of various processor execution units
//! including the ALU, FPU, branch prediction unit, load/store unit, memory
//! management unit, cache system, and prefetchers.

/// Arithmetic Logic Unit for integer operations.
pub mod alu;

/// Branch Resolution Unit including branch predictors and BTB.
pub mod bru;

/// Cache hierarchy implementation (L1, L2, L3) with replacement policies.
pub mod cache;

/// Floating-Point Unit for IEEE 754 operations.
pub mod fpu;

/// Load/Store Unit for memory access operations.
pub mod lsu;

/// Memory Management Unit with TLB and page table walker.
pub mod mmu;

/// Hardware prefetcher implementations (stride, stream, tagged).
pub mod prefetch;

//! Tracing macros for every pipeline subsystem.
//!
//! Each macro takes a runtime guard (`cpu.trace`) as a coarse on/off switch,
//! then delegates to the `tracing` crate. The `tracing` subscriber uses a
//! static `AtomicBool` per callsite — when no subscriber is listening the
//! check is a single relaxed load, branch-predicted-not-taken after warmup,
//! with zero string formatting or allocation.
//!
//! For finer control, `RUST_LOG` env-filter applies on top of the guard:
//!
//! ```bash
//! RUST_LOG=rvsim::execute=trace,rvsim::commit=trace ./rvsim ...
//! RUST_LOG=rvsim=trace ./rvsim ...                       # everything
//! RUST_LOG=rvsim::mem=trace,rvsim::fwd=trace ./rvsim ... # memory only
//! ```
//!
//! | Macro              | Target              | Covers |
//! |--------------------|---------------------|--------|
//! | `trace_fetch!`     | `rvsim::fetch`      | F1/F2: PC gen, I-TLB, I-cache, branch prediction |
//! | `trace_decode!`    | `rvsim::decode`     | Decode: raw bits, RVC expansion, illegal inst |
//! | `trace_rename!`    | `rvsim::rename`     | Rename: arch→phys mapping, ROB/SB alloc |
//! | `trace_issue!`     | `rvsim::issue`      | Issue queue: wakeup, select, FU hazards |
//! | `trace_execute!`   | `rvsim::execute`    | Execute: operands, result, branch resolution |
//! | `trace_mem!`       | `rvsim::mem`        | Mem1/Mem2: translation, cache, MSHR |
//! | `trace_writeback!` | `rvsim::writeback`  | Writeback: ROB complete, result |
//! | `trace_commit!`    | `rvsim::commit`     | Commit: retire, reg write, CSR, trap |
//! | `trace_branch!`    | `rvsim::branch`     | Branch predictor: predict, outcome, flush |
//! | `trace_trap!`      | `rvsim::trap`       | Trap/interrupt entry and return |
//! | `trace_csr!`       | `rvsim::csr`        | CSR reads/writes with before/after values |
//! | `trace_fwd!`       | `rvsim::fwd`        | Store-to-load forwarding, ordering violations |

// ---------------------------------------------------------------------------
// Fetch (F1 + F2): PC gen, I-TLB, I-cache misses, branch prediction events
// ---------------------------------------------------------------------------

/// Trace event for the Fetch1 / Fetch2 pipeline stages.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::fetch=trace`.
/// Target: `rvsim::fetch`
#[macro_export]
macro_rules! trace_fetch {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::fetch", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Decode: raw instruction bits, RVC expansion, illegal instruction detection
// ---------------------------------------------------------------------------

/// Trace event for the Decode pipeline stage.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::decode=trace`.
/// Target: `rvsim::decode`
#[macro_export]
macro_rules! trace_decode {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::decode", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Rename: arch→phys register mapping, ROB/SB/LQ alloc, free list state
// ---------------------------------------------------------------------------

/// Trace event for the Rename pipeline stage.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::rename=trace`.
/// Target: `rvsim::rename`
#[macro_export]
macro_rules! trace_rename {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::rename", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Issue: wakeup broadcast, select (oldest-ready), FU structural hazards
// ---------------------------------------------------------------------------

/// Trace event for the Issue Queue (wakeup/select).
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::issue=trace`.
/// Target: `rvsim::issue`
#[macro_export]
macro_rules! trace_issue {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::issue", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Execute: operand values, ALU/FPU result, branch/jump resolution
// ---------------------------------------------------------------------------

/// Trace event for the Execute pipeline stage.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::execute=trace`.
/// Target: `rvsim::execute`
#[macro_export]
macro_rules! trace_execute {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::execute", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Memory (Mem1 + Mem2): VA→PA translation, cache access, MSHR, forwarding
// ---------------------------------------------------------------------------

/// Trace event for the Memory1 and Memory2 pipeline stages.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::mem=trace`.
/// Target: `rvsim::mem`
#[macro_export]
macro_rules! trace_mem {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::mem", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Writeback: ROB entry marked complete, result value
// ---------------------------------------------------------------------------

/// Trace event for the Writeback pipeline stage.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::writeback=trace`.
/// Target: `rvsim::writeback`
#[macro_export]
macro_rules! trace_writeback {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::writeback", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Commit: instruction retirement, register writes, CSR application, traps
// ---------------------------------------------------------------------------

/// Trace event for the Commit pipeline stage.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::commit=trace`.
/// Target: `rvsim::commit`
#[macro_export]
macro_rules! trace_commit {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::commit", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Branch: prediction made, outcome, misprediction flush
// ---------------------------------------------------------------------------

/// Trace event for branch prediction events.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::branch=trace`.
/// Target: `rvsim::branch`
#[macro_export]
macro_rules! trace_branch {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::branch", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Trap: exception / interrupt entry, MRET/SRET return
// ---------------------------------------------------------------------------

/// Trace event for trap and interrupt handling.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::trap=trace`.
/// Target: `rvsim::trap`
#[macro_export]
macro_rules! trace_trap {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::trap", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// CSR: reads and writes with before/after values
// ---------------------------------------------------------------------------

/// Trace event for CSR reads and writes.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::csr=trace`.
/// Target: `rvsim::csr`
#[macro_export]
macro_rules! trace_csr {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::csr", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Forwarding: store-to-load forwarding hits and ordering violations
// ---------------------------------------------------------------------------

/// Trace event for store-to-load forwarding and memory ordering violations.
///
/// Enabled by the runtime guard (`cpu.trace`) + `RUST_LOG=rvsim::fwd=trace`.
/// Target: `rvsim::fwd`
#[macro_export]
macro_rules! trace_fwd {
    ($guard:expr; $($arg:tt)*) => {
        if $guard {
            ::tracing::trace!(target: "rvsim::fwd", $($arg)*)
        }
    };
}

// ---------------------------------------------------------------------------
// Helper: format a u64 as "0x{:016x}" without allocating a String.
// Use inside trace macros as: addr = %crate::trace::hex(some_u64)
// ---------------------------------------------------------------------------

/// Wraps a `u64` so it formats as `0x0000000000000000` in tracing fields.
///
/// # Example
/// ```ignore
/// trace_execute!(cpu.trace; pc = %crate::trace::Hex(entry.pc), result = %crate::trace::Hex(alu_out), "EX");
/// ```
#[derive(Debug)]
pub struct Hex(pub u64);

impl std::fmt::Display for Hex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#018x}", self.0)
    }
}

/// Wraps a `u32` so it formats as `0x00000000` in tracing fields.
#[derive(Debug)]
pub struct Hex32(pub u32);

impl std::fmt::Display for Hex32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#010x}", self.0)
    }
}

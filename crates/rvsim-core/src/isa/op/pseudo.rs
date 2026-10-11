//! The pseudo-operations a program asks the simulator for.
//!
//! They ride gem5's m5op instruction: opcode `0x7b` (custom-3), the
//! operation in bits 31:25, its arguments in `a0` to `a3` and, for the
//! two that return one, its result in `a0`. The numbers are gem5's, so a
//! binary instrumented with gem5's libm5 runs unchanged, and
//! `software/include/rvsim.h` emits the same encodings.

/// The m5op opcode, custom-3.
pub const OPCODE: u32 = 0b111_1011;

/// What an m5op asks for, by gem5's function number
/// (`include/gem5/asm/generic/m5ops.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PseudoOp {
    /// `m5_exit`: the run ends with exit code 0.
    Exit,
    /// `m5_fail(delay, code)`: the run ends with exit code `a1`.
    Fail,
    /// `m5_reset_stats`: the stats window starts again here.
    ResetStats,
    /// `m5_dump_stats`: a snapshot of the stats, labelled `a0`.
    DumpStats,
    /// `m5_dump_reset_stats`: a snapshot labelled `a0`, then a reset.
    DumpResetStats,
    /// `m5_checkpoint`: the host's run stops here, labelled `a0`, for it
    /// to save.
    Checkpoint,
    /// `m5_work_begin(id, thread)`: a marker the host's run stops at,
    /// labelled `a0`.
    WorkBegin,
    /// `m5_work_end(id, thread)`: a marker the host's run stops at,
    /// labelled `a0`.
    WorkEnd,
    /// `m5_write_file(buf, len, offset, name)`: `a1` bytes of the
    /// program's memory at `a0` go to offset `a2` of the file named by the
    /// string at `a3` in the host's output directory; `a0` gets how many.
    WriteFile,
    /// `m5_read_file(buf, len, offset)`: up to `a1` bytes of the host's
    /// script file from offset `a2` go to the program's memory at `a0`;
    /// `a0` gets how many, 0 at the end of the file.
    ReadFile,
    /// An m5op rvsim does not perform.
    Unknown(u32),
}

impl PseudoOp {
    /// The operation the instruction word `inst` asks for.
    #[must_use]
    pub const fn of(inst: u32) -> Self {
        match inst >> 25 {
            0x21 => Self::Exit,
            0x22 => Self::Fail,
            0x40 => Self::ResetStats,
            0x41 => Self::DumpStats,
            0x42 => Self::DumpResetStats,
            0x43 => Self::Checkpoint,
            0x4f => Self::WriteFile,
            0x50 => Self::ReadFile,
            0x5a => Self::WorkBegin,
            0x5b => Self::WorkEnd,
            func => Self::Unknown(func),
        }
    }

    /// Whether the operation leaves a result in `a0`, which is then the
    /// instruction's destination.
    #[must_use]
    pub const fn returns_value(self) -> bool {
        matches!(self, Self::WriteFile | Self::ReadFile)
    }

    /// The instruction word for gem5's function number `func`.
    #[must_use]
    pub const fn encode(func: u32) -> u32 {
        OPCODE | (func << 25)
    }
}

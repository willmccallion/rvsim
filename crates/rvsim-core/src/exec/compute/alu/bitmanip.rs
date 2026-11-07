//! ALU bit-manipulation operations (B extension: Zba, Zbb, Zbc, Zbs, and
//! the Zbkb/Zbkx packing and permutes).
//!
//! Reference: RISC-V Bitmanip Extension v1.0.0.

use crate::exec::compute::clmul::{clmul_high, clmul_low};
use crate::isa::bits::{sext_word, zext_word};
use crate::isa::op::{AluOp, IntWidth};

/// Executes a B-extension ALU operation on `a` (rs1) and `b` (rs2 or the
/// immediate). A `Word` operation's result is sign-extended from bit 31.
/// Returns `0` for an op outside the extension.
pub fn execute(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        AluOp::Sh1Add
        | AluOp::Sh2Add
        | AluOp::Sh3Add
        | AluOp::AddUw
        | AluOp::Sh1AddUw
        | AluOp::Sh2AddUw
        | AluOp::Sh3AddUw
        | AluOp::SlliUw => address(op, a, b),
        AluOp::Andn => a & !b,
        AluOp::Orn => a | !b,
        AluOp::Xnor => !(a ^ b),
        AluOp::Clz | AluOp::Ctz | AluOp::Cpop => count_bits(op, a, width),
        AluOp::Max | AluOp::Maxu | AluOp::Min | AluOp::Minu => min_max(op, a, b, width),
        AluOp::SextB => a as i8 as i64 as u64,
        AluOp::SextH => a as i16 as i64 as u64,
        AluOp::Rol | AluOp::Ror => rotate(op, a, b, width),
        AluOp::OrcB => map_bytes(a, |byte| if byte == 0 { 0 } else { 0xFF }),
        AluOp::Rev8 => a.swap_bytes(),
        AluOp::Brev8 => a.reverse_bits().swap_bytes(),
        AluOp::Clmul => clmul_low(a, b, 64),
        AluOp::Clmulh => clmul_high(a, b, 64),
        // Bits 126..63 of the 128-bit product.
        AluOp::Clmulr => (clmul_high(a, b, 64) << 1) | (clmul_low(a, b, 64) >> 63),
        AluOp::Bclr | AluOp::Bext | AluOp::Binv | AluOp::Bset => single_bit(op, a, b, width),
        AluOp::Pack | AluOp::Packh | AluOp::Packw => pack(op, a, b, width),
        AluOp::Xperm4 => crossbar_permute(a, b, 4),
        AluOp::Xperm8 => crossbar_permute(a, b, 8),
        _ => 0,
    }
}

/// Zba's address generation: `a`, zero-extended from a word for the `.uw`
/// forms, shifted and added to `b`, or shifted by `b` for `slli.uw`.
const fn address(op: AluOp, a: u64, b: u64) -> u64 {
    match op {
        AluOp::Sh1Add => (a << 1).wrapping_add(b),
        AluOp::Sh2Add => (a << 2).wrapping_add(b),
        AluOp::Sh3Add => (a << 3).wrapping_add(b),
        AluOp::AddUw => zext_word(a).wrapping_add(b),
        AluOp::Sh1AddUw => (zext_word(a) << 1).wrapping_add(b),
        AluOp::Sh2AddUw => (zext_word(a) << 2).wrapping_add(b),
        AluOp::Sh3AddUw => (zext_word(a) << 3).wrapping_add(b),
        AluOp::SlliUw => zext_word(a) << (b & shamt_mask(IntWidth::Double) as u64),
        _ => 0,
    }
}

/// The shift amounts a width uses: 5 bits for a word, 6 for a doubleword.
const fn shamt_mask(width: IntWidth) -> u32 {
    match width {
        IntWidth::Word => 0x1F,
        IntWidth::Double => 0x3F,
    }
}

/// `clz`, `ctz` or `cpop` over the low word or the whole register.
const fn count_bits(op: AluOp, a: u64, width: IntWidth) -> u64 {
    let word = a as u32;
    let count = match (op, width) {
        (AluOp::Clz, IntWidth::Word) => word.leading_zeros(),
        (AluOp::Clz, IntWidth::Double) => a.leading_zeros(),
        (AluOp::Ctz, IntWidth::Word) => word.trailing_zeros(),
        (AluOp::Ctz, IntWidth::Double) => a.trailing_zeros(),
        (AluOp::Cpop, IntWidth::Word) => word.count_ones(),
        (AluOp::Cpop, IntWidth::Double) => a.count_ones(),
        _ => 0,
    };
    count as u64
}

/// `min`, `max` and their unsigned forms. A word compares sign-extended,
/// which keeps both its signed and its unsigned order and is already the
/// register image of the result.
fn min_max(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let (a, b) = match width {
        IntWidth::Word => (sext_word(a as u32), sext_word(b as u32)),
        IntWidth::Double => (a, b),
    };
    match op {
        AluOp::Max => (a as i64).max(b as i64) as u64,
        AluOp::Min => (a as i64).min(b as i64) as u64,
        AluOp::Maxu => a.max(b),
        AluOp::Minu => a.min(b),
        _ => 0,
    }
}

/// `rol` and `ror` over the low word or the whole register.
const fn rotate(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let shamt = b as u32 & shamt_mask(width);
    match (op, width) {
        (AluOp::Rol, IntWidth::Word) => sext_word((a as u32).rotate_left(shamt)),
        (AluOp::Rol, IntWidth::Double) => a.rotate_left(shamt),
        (AluOp::Ror, IntWidth::Word) => sext_word((a as u32).rotate_right(shamt)),
        (AluOp::Ror, IntWidth::Double) => a.rotate_right(shamt),
        _ => 0,
    }
}

/// Zbs: clear, extract, invert or set the bit of `a` that `b` numbers.
const fn single_bit(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let bit = 1u64 << (b as u32 & shamt_mask(width));
    match op {
        AluOp::Bclr => a & !bit,
        AluOp::Bext => (a & bit != 0) as u64,
        AluOp::Binv => a ^ bit,
        AluOp::Bset => a | bit,
        _ => 0,
    }
}

/// Zbkb's packing of the low halves (or bytes for `packh`) of `a` and `b`.
const fn pack(op: AluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match (op, width) {
        (AluOp::Pack, IntWidth::Double) => zext_word(a) | (zext_word(b) << 32),
        (AluOp::Pack, IntWidth::Word) | (AluOp::Packw, _) => {
            sext_word((a as u16 as u32) | ((b as u16 as u32) << 16))
        }
        (AluOp::Packh, _) => (a & 0xFF) | ((b & 0xFF) << 8),
        _ => 0,
    }
}

/// `a` with each byte replaced by `f` of it.
fn map_bytes(a: u64, f: impl Fn(u8) -> u8) -> u64 {
    u64::from_le_bytes(a.to_le_bytes().map(f))
}

/// Zbkx's crossbar permute: each `lane_bits`-wide lane of `b` selects a
/// lane of `a` by index, or zero when the index is past the last lane.
fn crossbar_permute(a: u64, b: u64, lane_bits: u32) -> u64 {
    let lanes = 64 / lane_bits;
    let lane_mask = (1u64 << lane_bits) - 1;
    (0..lanes).fold(0, |result, lane| {
        let index = ((b >> (lane * lane_bits)) & lane_mask) as u32;
        let selected = if index < lanes { (a >> (index * lane_bits)) & lane_mask } else { 0 };
        result | (selected << (lane * lane_bits))
    })
}

//! ALU bit-manipulation operations (B extension: Zba, Zbb, Zbc, Zbs, and
//! the Zbkb/Zbkx packing and permutes).
//!
//! Reference: RISC-V Bitmanip Extension v1.0.0.

use crate::exec::compute::clmul::{clmul_high, clmul_low};
use crate::isa::bits::{sext_word, zext_word};
use crate::isa::op::{
    BitOp, ClmulOp, CountOp, IntWidth, MinMaxOp, PackOp, RotateOp, ShiftAddOp, SingleBitOp,
};

/// Executes a B-extension ALU operation on `a` (rs1) and `b` (rs2 or the
/// immediate). A `Word` operation's result is sign-extended from bit 31.
pub fn execute(op: BitOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        BitOp::ShiftAdd(op) => shift_add(op, a, b),
        BitOp::Andn => a & !b,
        BitOp::Orn => a | !b,
        BitOp::Xnor => !(a ^ b),
        BitOp::Count(op) => count_bits(op, a, width),
        BitOp::MinMax(op) => min_max(op, a, b, width),
        BitOp::SextB => i64::from(a as i8).cast_unsigned(),
        BitOp::SextH => i64::from(a as i16).cast_unsigned(),
        BitOp::Rotate(op) => rotate(op, a, b, width),
        BitOp::OrcB => map_bytes(a, |byte| if byte == 0 { 0 } else { 0xFF }),
        BitOp::Rev8 => a.swap_bytes(),
        BitOp::Brev8 => a.reverse_bits().swap_bytes(),
        BitOp::Clmul(op) => carry_less_multiply(op, a, b),
        BitOp::SingleBit(op) => single_bit(op, a, b, width),
        BitOp::Pack(op) => pack(op, a, b, width),
        BitOp::Xperm4 => crossbar_permute(a, b, 4),
        BitOp::Xperm8 => crossbar_permute(a, b, 8),
    }
}

/// Zba's address generation: `a`, zero-extended from a word for the `.uw`
/// forms, shifted and added to `b`, or shifted by `b` for `slli.uw`.
const fn shift_add(op: ShiftAddOp, a: u64, b: u64) -> u64 {
    match op {
        ShiftAddOp::Sh1Add => (a << 1).wrapping_add(b),
        ShiftAddOp::Sh2Add => (a << 2).wrapping_add(b),
        ShiftAddOp::Sh3Add => (a << 3).wrapping_add(b),
        ShiftAddOp::AddUw => zext_word(a).wrapping_add(b),
        ShiftAddOp::Sh1AddUw => (zext_word(a) << 1).wrapping_add(b),
        ShiftAddOp::Sh2AddUw => (zext_word(a) << 2).wrapping_add(b),
        ShiftAddOp::Sh3AddUw => (zext_word(a) << 3).wrapping_add(b),
        ShiftAddOp::SlliUw => zext_word(a) << (b & shamt_mask(IntWidth::Double) as u64),
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
const fn count_bits(op: CountOp, a: u64, width: IntWidth) -> u64 {
    let word = a as u32;
    let count = match (op, width) {
        (CountOp::Clz, IntWidth::Word) => word.leading_zeros(),
        (CountOp::Clz, IntWidth::Double) => a.leading_zeros(),
        (CountOp::Ctz, IntWidth::Word) => word.trailing_zeros(),
        (CountOp::Ctz, IntWidth::Double) => a.trailing_zeros(),
        (CountOp::Cpop, IntWidth::Word) => word.count_ones(),
        (CountOp::Cpop, IntWidth::Double) => a.count_ones(),
    };
    count as u64
}

/// `min`, `max` and their unsigned forms. A word compares sign-extended,
/// which keeps both its signed and its unsigned order and is already the
/// register image of the result.
fn min_max(op: MinMaxOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let (a, b) = match width {
        IntWidth::Word => (sext_word(a as u32), sext_word(b as u32)),
        IntWidth::Double => (a, b),
    };
    match op {
        MinMaxOp::Max => a.cast_signed().max(b.cast_signed()).cast_unsigned(),
        MinMaxOp::Min => a.cast_signed().min(b.cast_signed()).cast_unsigned(),
        MinMaxOp::Maxu => a.max(b),
        MinMaxOp::Minu => a.min(b),
    }
}

/// `rol` and `ror` over the low word or the whole register.
const fn rotate(op: RotateOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let shamt = b as u32 & shamt_mask(width);
    match (op, width) {
        (RotateOp::Rol, IntWidth::Word) => sext_word((a as u32).rotate_left(shamt)),
        (RotateOp::Rol, IntWidth::Double) => a.rotate_left(shamt),
        (RotateOp::Ror, IntWidth::Word) => sext_word((a as u32).rotate_right(shamt)),
        (RotateOp::Ror, IntWidth::Double) => a.rotate_right(shamt),
    }
}

/// Zbc's carry-less multiplies of the full registers.
const fn carry_less_multiply(op: ClmulOp, a: u64, b: u64) -> u64 {
    match op {
        ClmulOp::Clmul => clmul_low(a, b, 64),
        ClmulOp::Clmulh => clmul_high(a, b, 64),
        // Bits 126..63 of the 128-bit product.
        ClmulOp::Clmulr => (clmul_high(a, b, 64) << 1) | (clmul_low(a, b, 64) >> 63),
    }
}

/// Zbs: clear, extract, invert or set the bit of `a` that `b` numbers.
const fn single_bit(op: SingleBitOp, a: u64, b: u64, width: IntWidth) -> u64 {
    let bit = 1u64 << (b as u32 & shamt_mask(width));
    match op {
        SingleBitOp::Bclr => a & !bit,
        SingleBitOp::Bext => (a & bit != 0) as u64,
        SingleBitOp::Binv => a ^ bit,
        SingleBitOp::Bset => a | bit,
    }
}

/// Zbkb's packing of the low halves (or bytes for `packh`) of `a` and `b`.
const fn pack(op: PackOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match (op, width) {
        (PackOp::Pack, IntWidth::Double) => zext_word(a) | (zext_word(b) << 32),
        (PackOp::Pack, IntWidth::Word) | (PackOp::Packw, _) => {
            sext_word((a as u16 as u32) | ((b as u16 as u32) << 16))
        }
        (PackOp::Packh, _) => (a & 0xFF) | ((b & 0xFF) << 8),
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

//! Arithmetic Logic Unit (ALU).
//!
//! This module implements the integer ALU used in the Execute stage.
//! It handles standard arithmetic, logical operations, and shifts
//! for both 32-bit and 64-bit operands. It also implements the
//! Multiply/Divide (M) and Bit-Manipulation (B) extension operations.
//!
//! Operations are organized into submodules by category:
//! - [`arithmetic`]: add and subtract, the M extension's multiply and divide
//! - [`logic`]:      Or, And, Xor, Slt, Sltu
//! - [`shifts`]:     Sll, Srl, Sra
//! - [`bitmanip`]:   Zba (address gen), Zbb (basic bitmanip), Zbc (clmul), Zbs (single-bit)

pub mod arithmetic;

pub mod bitmanip;

pub mod logic;

pub mod shifts;

use crate::isa::op::{IntAluOp, IntWidth};

/// Executes an integer ALU operation on `a` (rs1) and `b` (rs2 or the
/// immediate). A `Word` operation's result is sign-extended from bit 31.
pub fn execute(op: IntAluOp, a: u64, b: u64, width: IntWidth) -> u64 {
    match op {
        IntAluOp::Arith(op) => arithmetic::add_sub(op, a, b, width),
        IntAluOp::Mul(op) => arithmetic::multiply(op, a, b, width),
        IntAluOp::Div(op) => arithmetic::divide(op, a, b, width),
        IntAluOp::Logic(op) => logic::execute(op, a, b, width),
        IntAluOp::Shift(op) => shifts::execute(op, a, b, width),
        IntAluOp::Bit(op) => bitmanip::execute(op, a, b, width),
    }
}

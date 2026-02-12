//! Arithmetic Logic Unit (ALU).
//!
//! This module implements the integer ALU used in the Execute stage.
//! It handles standard arithmetic, logical operations, and shifts
//! for both 32-bit and 64-bit operands. It also implements the
//! Multiply/Divide (M) extension operations.
//!
//! Operations are organized into submodules by category:
//! - [`arithmetic`]: Add, Sub, Mul, Mulh, Mulhsu, Mulhu, Div, Divu, Rem, Remu
//! - [`logic`]:      Or, And, Xor, Slt, Sltu
//! - [`shifts`]:     Sll, Srl, Sra

/// Integer arithmetic operations (add, subtract, multiply, divide).
pub mod arithmetic;

/// Bitwise logical and comparison operations (or, and, xor, slt).
pub mod logic;

/// Shift operations (sll, srl, sra).
pub mod shifts;

use crate::core::pipeline::signals::AluOp;

/// Arithmetic Logic Unit (ALU) for integer operations.
///
/// Implements all RISC-V integer arithmetic and logical operations
/// including addition, subtraction, shifts, comparisons, and
/// multiply/divide operations from the I and M extensions.
pub struct Alu;

impl Alu {
    /// Executes an integer ALU operation.
    ///
    /// Dispatches to the appropriate submodule based on the operation type.
    /// Supports both 32-bit and 64-bit operations based on the `is32` flag.
    ///
    /// # Arguments
    ///
    /// * `op`   - The ALU operation to perform
    /// * `a`    - First operand (64-bit value)
    /// * `b`    - Second operand (64-bit value, also used as shift amount)
    /// * `_c`   - Third operand (currently unused, reserved for future use)
    /// * `is32` - If true, perform 32-bit operation (RV32 mode)
    ///
    /// # Returns
    ///
    /// The 64-bit result of the ALU operation. For 32-bit operations,
    /// the result is sign-extended to 64 bits.
    ///
    /// # Examples
    ///
    /// ```
    /// use riscv_core::core::units::alu::Alu;
    /// use riscv_core::core::pipeline::signals::AluOp;
    ///
    /// // 64-bit addition
    /// let result = Alu::execute(AluOp::Add, 42, 8, 0, false);
    /// assert_eq!(result, 50);
    ///
    /// // 32-bit addition with sign extension
    /// let result = Alu::execute(AluOp::Add, 0xFFFFFFFF, 1, 0, true);
    /// assert_eq!(result, 0); // Wraps to 0 and sign-extends
    ///
    /// // Logical shift left
    /// let result = Alu::execute(AluOp::Sll, 0x1, 4, 0, false);
    /// assert_eq!(result, 0x10);
    ///
    /// // Signed comparison
    /// let result = Alu::execute(AluOp::Slt, -5_i64 as u64, 10, 0, false);
    /// assert_eq!(result, 1); // -5 < 10
    ///
    /// // Unsigned division
    /// let result = Alu::execute(AluOp::Divu, 100, 7, 0, false);
    /// assert_eq!(result, 14);
    /// ```
    pub fn execute(op: AluOp, a: u64, b: u64, _c: u64, is32: bool) -> u64 {
        match op {
            // Arithmetic: add, sub, mul*, div*, rem*
            AluOp::Add
            | AluOp::Sub
            | AluOp::Mul
            | AluOp::Mulh
            | AluOp::Mulhsu
            | AluOp::Mulhu
            | AluOp::Div
            | AluOp::Divu
            | AluOp::Rem
            | AluOp::Remu => arithmetic::execute(op, a, b, is32),

            // Logic / comparisons: or, and, xor, slt, sltu
            AluOp::Or | AluOp::And | AluOp::Xor | AluOp::Slt | AluOp::Sltu => {
                logic::execute(op, a, b, is32)
            }

            // Shifts: sll, srl, sra
            AluOp::Sll | AluOp::Srl | AluOp::Sra => shifts::execute(op, a, b, is32),

            // Non-integer operations (FP, etc.) are not handled here.
            _ => 0,
        }
    }
}

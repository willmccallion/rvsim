//! The ALU and FPU operations an instruction decodes to.

/// ALU operation types for integer and floating-point instructions.
#[derive(Clone, Copy, Debug, Default)]
pub enum AluOp {
    /// Default value (no operation).
    #[default]
    Add,

    /// Integer subtraction.
    Sub,

    /// Shift left logical.
    Sll,

    /// Set less than (signed).
    Slt,

    /// Set less than unsigned.
    Sltu,

    /// Bitwise XOR.
    Xor,

    /// Shift right logical.
    Srl,

    /// Shift right arithmetic.
    Sra,

    /// Bitwise OR.
    Or,

    /// Bitwise AND.
    And,

    /// Integer multiply (low bits).
    Mul,

    /// Integer multiply (high bits, signed × signed).
    Mulh,

    /// Integer multiply (high bits, signed × unsigned).
    Mulhsu,

    /// Integer multiply (high bits, unsigned × unsigned).
    Mulhu,

    /// Integer divide (signed).
    Div,

    /// Integer divide (unsigned).
    Divu,

    /// Integer remainder (signed).
    Rem,

    /// Integer remainder (unsigned).
    Remu,

    /// Floating-point addition.
    FAdd,

    /// Floating-point subtraction.
    FSub,

    /// Floating-point multiplication.
    FMul,

    /// Floating-point division.
    FDiv,

    /// Floating-point square root.
    FSqrt,

    /// Floating-point minimum.
    FMin,

    /// Floating-point maximum.
    FMax,

    /// Floating-point multiply-add (fused).
    FMAdd,

    /// Floating-point multiply-subtract (fused).
    FMSub,

    /// Floating-point negated multiply-add (fused).
    FNMAdd,

    /// Floating-point negated multiply-subtract (fused).
    FNMSub,

    /// Convert word to single-precision float (signed).
    FCvtWS,

    /// Convert long to single-precision float (signed).
    FCvtLS,

    /// Convert single-precision float to word (signed).
    FCvtSW,

    /// Convert single-precision float to long (signed).
    FCvtSL,

    /// Convert float to word (unsigned).
    FCvtWUS,

    /// Convert float to long (unsigned).
    FCvtLUS,

    /// Convert unsigned word to float.
    FCvtSWU,

    /// Convert unsigned long to float.
    FCvtSLU,

    /// Convert single-precision to double-precision float.
    FCvtSD,

    /// Convert double-precision to single-precision float.
    FCvtDS,

    /// Convert half-precision to single-precision float (Zfh).
    FCvtSH,

    /// Convert single-precision to half-precision float (Zfh).
    FCvtHS,

    /// Convert half-precision to double-precision float (Zfh).
    FCvtDH,

    /// Convert double-precision to half-precision float (Zfh).
    FCvtHD,

    /// Floating-point sign injection (copy sign).
    FSgnJ,

    /// Floating-point sign injection (negate sign).
    FSgnJN,

    /// Floating-point sign injection (XOR sign).
    FSgnJX,

    /// Floating-point equality comparison.
    FEq,

    /// Floating-point less-than comparison.
    FLt,

    /// Floating-point less-than-or-equal comparison.
    FLe,

    /// Floating-point classify.
    FClass,

    /// Move floating-point register to integer register.
    FMvToX,

    /// Move integer register to floating-point register.
    FMvToF,

    /// Shift-left-1 and add (sh1add).
    Sh1Add,

    /// Shift-left-2 and add (sh2add).
    Sh2Add,

    /// Shift-left-3 and add (sh3add).
    Sh3Add,

    /// Add unsigned word (add.uw) — zero-extends rs1\[31:0\] before adding.
    AddUw,

    /// Shift-left-1 and add unsigned word (sh1add.uw).
    Sh1AddUw,

    /// Shift-left-2 and add unsigned word (sh2add.uw).
    Sh2AddUw,

    /// Shift-left-3 and add unsigned word (sh3add.uw).
    Sh3AddUw,

    /// Shift-left-logical unsigned word immediate (slli.uw).
    SlliUw,

    /// Bitwise AND with complement (andn).
    Andn,

    /// Bitwise OR with complement (orn).
    Orn,

    /// Bitwise exclusive NOR (xnor).
    Xnor,

    /// Count leading zeros (clz / clzw).
    Clz,

    /// Count trailing zeros (ctz / ctzw).
    Ctz,

    /// Count set bits / population count (cpop / cpopw).
    Cpop,

    /// Maximum (signed).
    Max,

    /// Maximum (unsigned).
    Maxu,

    /// Minimum (signed).
    Min,

    /// Minimum (unsigned).
    Minu,

    /// Sign-extend byte.
    SextB,

    /// Sign-extend halfword.
    SextH,

    /// Rotate left (rol / rolw).
    Rol,

    /// Rotate right (ror / rorw / rori / roriw).
    Ror,

    /// OR-combine bytes (orc.b).
    OrcB,

    /// Byte-reverse (rev8).
    Rev8,

    /// Carry-less multiply (low half).
    Clmul,

    /// Carry-less multiply (high half).
    Clmulh,

    /// Carry-less multiply (reversed / remainder).
    Clmulr,

    /// Clear single bit (bclr / bclri).
    Bclr,

    /// Extract single bit (bext / bexti).
    Bext,

    /// Invert single bit (binv / binvi).
    Binv,

    /// Set single bit (bset / bseti).
    Bset,

    /// Bit-reverse within each byte (brev8).
    Brev8,

    /// Pack lower halves of two registers (pack).
    Pack,

    /// Pack lowest bytes of two registers (packh).
    Packh,

    /// Pack lower halves, 32-bit variant (packw).
    Packw,

    /// 4-bit crossbar permutation (xperm4).
    Xperm4,

    /// 8-bit crossbar permutation (xperm8).
    Xperm8,
}

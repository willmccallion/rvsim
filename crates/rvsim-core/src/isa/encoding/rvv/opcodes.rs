//! RVV opcode constants.

/// Vector arithmetic instructions (OP-V).
pub const OP_V: u32 = 0b1010111;

/// Vector cryptography instructions (OP-V-CRYPTO).
///
/// Used by Zvkned (AES), Zvknha/b (SHA-2), Zvksed (SM4), Zvksh (SM3), and
/// Zvkg (GHASH/GMAC). All Zvk* ops use this opcode with `funct3 = 0b010`
/// (OPMVV) and `funct6` values in 0x20..0x2F (with bit 25 always 1 —
/// unmasked).
pub const OP_V_CRYPTO: u32 = 0b1110111;

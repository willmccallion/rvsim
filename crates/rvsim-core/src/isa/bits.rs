//! Sign and zero extension of a narrow value held in a wider integer.

/// The low `bits` bits of `value`, sign-extended.
#[must_use]
pub const fn sign_extend(value: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    (value << shift).cast_signed() >> shift
}

/// A 32-bit result as RV64 writes it to a register: sign-extended.
#[must_use]
pub const fn sext_word(word: u32) -> u64 {
    (word.cast_signed() as i64).cast_unsigned()
}

/// The low 32 bits of `value`, zero-extended.
#[must_use]
pub const fn zext_word(value: u64) -> u64 {
    value & 0xFFFF_FFFF
}

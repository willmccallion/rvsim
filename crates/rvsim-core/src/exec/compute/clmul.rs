//! Carry-less (GF(2) polynomial) multiplication, shared by the scalar Zbc
//! and vector Zvbc units.

/// Carry-less multiply (GF(2)): low `bits` bits of the 2×`bits`-bit product.
///
/// For each set bit `i` in `a`, XOR `b << i` into the accumulator.
/// Equivalent to a polynomial multiply over GF(2).
#[inline]
pub const fn clmul_low(a: u64, b: u64, bits: usize) -> u64 {
    let mut acc: u64 = 0;
    let mut i = 0;
    while i < bits {
        if (a >> i) & 1 != 0 {
            acc ^= b << i;
        }
        i += 1;
    }
    acc
}

/// Carry-less multiply (GF(2)): high `bits` bits of the 2×`bits`-bit product.
#[inline]
pub const fn clmul_high(a: u64, b: u64, bits: usize) -> u64 {
    let mut acc: u64 = 0;
    let mut i = 1; // bit 0 of high half corresponds to bit `bits` of full product
    while i < bits {
        if (a >> i) & 1 != 0 {
            // b shifted left by `i`, then we want bits [bits .. 2*bits), so
            // shift right by `bits - i` to land the high half in `acc`.
            acc ^= b >> (bits - i);
        }
        i += 1;
    }
    acc
}

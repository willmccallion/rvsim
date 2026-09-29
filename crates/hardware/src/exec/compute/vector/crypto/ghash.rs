//! GHASH multiplication in GF(2^128) (Zvkg).

/// Reverse bits within each byte of a u32. Matches Spike's `ZVK_BREV8_32` macro.
#[inline]
pub(super) const fn brev8_u32(mut x: u32) -> u32 {
    x = ((x & 0x5555_5555) << 1) | ((x & 0xaaaa_aaaa) >> 1);
    x = ((x & 0x3333_3333) << 2) | ((x & 0xcccc_cccc) >> 2);
    x = ((x & 0x0f0f_0f0f) << 4) | ((x & 0xf0f0_f0f0) >> 4);
    x
}

/// Apply BREV8 to each lane of a [u32; 4] element group.
#[inline]
pub(super) const fn brev8_u32x4(x: [u32; 4]) -> [u32; 4] {
    [brev8_u32(x[0]), brev8_u32(x[1]), brev8_u32(x[2]), brev8_u32(x[3])]
}

/// `multiplier * multiplicand` in GF(2^128) using the GHASH (NIST GCM)
/// convention: BREV8 the operands, run a left-shift carry-less multiply
/// with reduction polynomial 0x87 (x^128 + x^7 + x^2 + x + 1), then BREV8
/// the result. Mirrors Spike's vgmul/vghsh inner loop verbatim.
pub(super) fn gf128_mul(multiplier: [u32; 4], multiplicand: [u32; 4]) -> [u32; 4] {
    let y = brev8_u32x4(multiplier);
    let mut h = brev8_u32x4(multiplicand);
    let mut z = [0u32; 4];

    for bit in 0..128 {
        let word = bit / 32;
        let bit_in_word = bit % 32;
        if (y[word] >> bit_in_word) & 1 == 1 {
            for i in 0..4 {
                z[i] ^= h[i];
            }
        }

        let reduce = (h[3] >> 31) & 1 == 1;
        // 128-bit left shift treating h as h[3]:h[2]:h[1]:h[0] (high → low).
        let h_full = (u128::from(h[3]) << 96)
            | (u128::from(h[2]) << 64)
            | (u128::from(h[1]) << 32)
            | u128::from(h[0]);
        let shifted = h_full << 1;
        h[0] = shifted as u32;
        h[1] = (shifted >> 32) as u32;
        h[2] = (shifted >> 64) as u32;
        h[3] = (shifted >> 96) as u32;
        if reduce {
            h[0] ^= 0x87;
        }
    }

    brev8_u32x4(z)
}

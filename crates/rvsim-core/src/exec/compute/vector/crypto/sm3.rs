//! SM3 message expansion and compression (Zvksh).

/// SM3 P0 / P1 / FF / GG functions per GB/T 32905-2016.
#[inline]
pub(super) const fn sm3_p0(x: u32) -> u32 {
    x ^ x.rotate_left(9) ^ x.rotate_left(17)
}

#[inline]
pub(super) const fn sm3_p1(x: u32) -> u32 {
    x ^ x.rotate_left(15) ^ x.rotate_left(23)
}

#[inline]
pub(super) const fn sm3_ff(x: u32, y: u32, z: u32, j: u32) -> u32 {
    if j < 16 { x ^ y ^ z } else { (x & y) | (x & z) | (y & z) }
}

#[inline]
pub(super) const fn sm3_gg(x: u32, y: u32, z: u32, j: u32) -> u32 {
    if j < 16 { x ^ y ^ z } else { (x & y) | (!x & z) }
}

#[inline]
pub(super) const fn sm3_t(j: u32) -> u32 {
    if j < 16 { 0x79CC4519 } else { 0x7A879D8A }
}

/// SM3 stores its state and message words byte-swapped in the vector
/// register relative to the algorithm's natural u32 representation. Both
/// extraction and storage apply `swap_bytes` per word to match spike's
/// `EXTRACT/SET_EGU32x8_WORDS_BE_BSWAP` macros.
#[inline]
pub(super) const fn bswap32(x: u32) -> u32 {
    x.swap_bytes()
}

/// vsm3me.vv — SM3 message expansion. Per Zvksh:
///   vs1 = {W7..W0}  (vs1[i] holds W[i] BSWAP'd)
///   vs2 = {W15..W8} (vs2[i] holds W[8+i] BSWAP'd)
/// Output replaces vd with {W23..W16}.
pub(super) fn sm3_me(vs1: [u32; 8], vs2: [u32; 8]) -> [u32; 8] {
    let mut w = [0u32; 24];
    for i in 0..8 {
        w[i] = bswap32(vs1[i]);
        w[i + 8] = bswap32(vs2[i]);
    }
    for j in 16..24 {
        w[j] = sm3_p1(w[j - 16] ^ w[j - 9] ^ w[j - 3].rotate_left(15))
            ^ w[j - 13].rotate_left(7)
            ^ w[j - 6];
    }
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = bswap32(w[16 + i]);
    }
    out
}

/// vsm3c.vi — SM3 compression. Per Zvksh:
///   vd  = {H,G,F,E,D,C,B,A}  (state, BSWAP'd; vd[0]=`A_bswap`, vd[7]=`H_bswap`)
///   vs2 = {_,_,w5,w4,_,_,w1,w0} (only positions 0,1,4,5 used)
/// Two rounds j = 2*rnd and j+1; output rewrites vd as
/// {G1, G2, E1, E2, C1, C2, A1, A2} (BSWAP'd).
pub(super) const fn sm3_c(vd: [u32; 8], vs2: [u32; 8], rnd: u32) -> [u32; 8] {
    let a = bswap32(vd[0]);
    let b = bswap32(vd[1]);
    let c = bswap32(vd[2]);
    let d = bswap32(vd[3]);
    let e = bswap32(vd[4]);
    let f = bswap32(vd[5]);
    let g = bswap32(vd[6]);
    let h = bswap32(vd[7]);

    let w0 = bswap32(vs2[0]);
    let w1 = bswap32(vs2[1]);
    let w4 = bswap32(vs2[4]);
    let w5 = bswap32(vs2[5]);
    let x0 = w0 ^ w4;
    let x1 = w1 ^ w5;

    // Round j = 2*rnd
    let j = 2 * rnd;
    let ss1 =
        a.rotate_left(12).wrapping_add(e).wrapping_add(sm3_t(j).rotate_left(j % 32)).rotate_left(7);
    let ss2 = ss1 ^ a.rotate_left(12);
    let tt1 = sm3_ff(a, b, c, j).wrapping_add(d).wrapping_add(ss2).wrapping_add(x0);
    let tt2 = sm3_gg(e, f, g, j).wrapping_add(h).wrapping_add(ss1).wrapping_add(w0);
    let d1 = c;
    let c1 = b.rotate_left(9);
    let b1 = a;
    let a1 = tt1;
    let h1 = g;
    let g1 = f.rotate_left(19);
    let f1 = e;
    let e1 = sm3_p0(tt2);

    // Round j+1
    let j = j + 1;
    let ss1 = a1
        .rotate_left(12)
        .wrapping_add(e1)
        .wrapping_add(sm3_t(j).rotate_left(j % 32))
        .rotate_left(7);
    let ss2 = ss1 ^ a1.rotate_left(12);
    let tt1 = sm3_ff(a1, b1, c1, j).wrapping_add(d1).wrapping_add(ss2).wrapping_add(x1);
    let tt2 = sm3_gg(e1, f1, g1, j).wrapping_add(h1).wrapping_add(ss1).wrapping_add(w1);
    let c2 = b1.rotate_left(9);
    let a2 = tt1;
    let g2 = f1.rotate_left(19);
    let e2 = sm3_p0(tt2);

    [
        bswap32(a2),
        bswap32(a1),
        bswap32(c2),
        bswap32(c1),
        bswap32(e2),
        bswap32(e1),
        bswap32(g2),
        bswap32(g1),
    ]
}
